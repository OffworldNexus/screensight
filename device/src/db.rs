//! SQLite persistence through SeaORM.
//!
//! Three tables: the single `device` row (identity), `instances` (paired Home
//! Assistant instances and which one is selected) and `kv` (per-instance
//! dashboard values). The connection is owned by a background writer task; the
//! runtime only sends it [`PersistCommand`]s, so writes never block the panel.
//!
//! The schema is created with `CREATE TABLE IF NOT EXISTS` on open. That is
//! enough for a device whose database is created fresh on first boot; a numbered
//! migration system can replace it if the schema ever needs to evolve.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use sea_orm::entity::prelude::*;
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{
    ConnectOptions, ConnectionTrait, Database, DatabaseBackend, DatabaseConnection, Set, Statement,
};
use sqlx::sqlite::{SqliteJournalMode, SqliteSynchronous};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::task::JoinHandle;

use crate::identity::{DeviceIdentity, DeviceKeys};
use crate::store::{PairedInstance, PersistCommand, Snapshot};

mod entity {
    //! SeaORM entities mirroring the three tables.

    pub mod device {
        use sea_orm::entity::prelude::*;

        /// The single persisted device row.
        #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
        #[sea_orm(table_name = "device")]
        pub struct Model {
            #[sea_orm(primary_key, auto_increment = false)]
            pub id: String,
            pub name: String,
            pub model: String,
            pub version: String,
            /// X25519 static private key, hex-encoded. Empty on a row that
            /// predates Noise; `load_snapshot` fills it in.
            pub noise_private: String,
            /// X25519 static public key, hex-encoded.
            pub noise_public: String,
        }

        #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
        pub enum Relation {}

        impl ActiveModelBehavior for ActiveModel {}
    }

    pub mod instance {
        use sea_orm::entity::prelude::*;

        /// One paired Home Assistant instance.
        #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
        #[sea_orm(table_name = "instances")]
        pub struct Model {
            #[sea_orm(primary_key, auto_increment = false)]
            pub ha_id: String,
            pub ha_name: String,
            /// Home Assistant's static Noise public key, hex-encoded.
            pub ha_static_key: String,
            pub last_ip: Option<String>,
            pub paired_at: i64,
            pub selected: bool,
        }

        #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
        pub enum Relation {}

        impl ActiveModelBehavior for ActiveModel {}
    }

    pub mod kv {
        use sea_orm::entity::prelude::*;

        /// One dashboard value for one instance.
        #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
        #[sea_orm(table_name = "kv")]
        pub struct Model {
            #[sea_orm(primary_key, auto_increment = false)]
            pub instance_id: String,
            #[sea_orm(primary_key, auto_increment = false)]
            pub key: String,
            pub value: String,
        }

        #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
        pub enum Relation {}

        impl ActiveModelBehavior for ActiveModel {}
    }
}

/// Current on-disk schema version (`PRAGMA user_version`). Bumped by [`migrate`].
///
/// Version 2 introduced the Noise static keys and dropped the bearer-token
/// pairing model. Because token pairings cannot be upgraded to key pairings,
/// migration discards `instances`/`kv` and forces a re-pair on Home Assistant;
/// the device identity row is preserved so its mDNS id and name survive.
const SCHEMA_VERSION: i64 = 2;

/// The base device table, in its pre-Noise shape. Migration adds the key columns.
const DEVICE_SCHEMA: &str = "CREATE TABLE IF NOT EXISTS device (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    model TEXT NOT NULL,
    version TEXT NOT NULL
)";

/// Paired instances, keyed by Home Assistant's static Noise public key.
const INSTANCES_SCHEMA: &str = "CREATE TABLE IF NOT EXISTS instances (
    ha_id TEXT PRIMARY KEY NOT NULL,
    ha_name TEXT NOT NULL,
    ha_static_key TEXT NOT NULL,
    last_ip TEXT,
    paired_at INTEGER NOT NULL,
    selected INTEGER NOT NULL DEFAULT 0
)";

/// Per-instance dashboard values.
const KV_SCHEMA: &str = "CREATE TABLE IF NOT EXISTS kv (
    instance_id TEXT NOT NULL,
    key TEXT NOT NULL,
    value TEXT NOT NULL,
    PRIMARY KEY (instance_id, key)
)";

/// Open (creating if needed) the database at `path` and ensure the schema.
pub async fn open(path: &Path) -> Result<DatabaseConnection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let url = format!("sqlite://{}?mode=rwc", path.display());
    connect(&url).await
}

async fn connect(url: &str) -> Result<DatabaseConnection> {
    let mut options = ConnectOptions::new(url.to_owned());
    // SQLite defaults to a rollback journal. WAL lets the background writer and
    // any read connection coexist without blocking, `NORMAL` is the durable
    // sweet spot for WAL (no corruption on power loss; only the most recent
    // transaction can be lost), and the busy timeout matches sqlx's default but
    // makes the intent explicit for the device's single-writer workload.
    options.map_sqlx_sqlite_opts(|opts| {
        opts.journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(Duration::from_secs(5))
            .foreign_keys(true)
    });
    let db = Database::connect(options)
        .await
        .with_context(|| format!("opening sqlite database {url}"))?;
    migrate(&db).await?;
    Ok(db)
}

/// Bring the database up to [`SCHEMA_VERSION`], creating the final schema on a
/// fresh install and destructively resetting an older one.
async fn migrate(db: &DatabaseConnection) -> Result<()> {
    let version: i64 = db
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Sqlite,
            "PRAGMA user_version;".to_owned(),
        ))
        .await
        .context("reading schema version")?
        .context("PRAGMA user_version returned no row")?
        .try_get_by_index(0)
        .context("decoding schema version")?;
    if version >= SCHEMA_VERSION {
        return Ok(());
    }

    // The device identity predates Noise: create the base table, then add the
    // key columns. Existing rows get empty keys that `load_snapshot` fills in.
    db.execute_unprepared(DEVICE_SCHEMA)
        .await
        .context("creating device table")?;
    add_column_if_missing(db, "device", "noise_private", "TEXT NOT NULL DEFAULT ''").await?;
    add_column_if_missing(db, "device", "noise_public", "TEXT NOT NULL DEFAULT ''").await?;

    // Token-based pairings are invalid under Noise; drop them and their values
    // rather than trying to invent static keys for them.
    db.execute_unprepared("DROP TABLE IF EXISTS instances")
        .await
        .context("dropping legacy instances")?;
    db.execute_unprepared("DROP TABLE IF EXISTS kv")
        .await
        .context("dropping legacy values")?;
    db.execute_unprepared(INSTANCES_SCHEMA)
        .await
        .context("creating instances table")?;
    db.execute_unprepared(KV_SCHEMA)
        .await
        .context("creating kv table")?;
    db.execute_unprepared(&format!("PRAGMA user_version = {SCHEMA_VERSION};"))
        .await
        .context("storing schema version")?;
    Ok(())
}

/// Add a column to `table` if `PRAGMA table_info` does not already list it.
async fn add_column_if_missing(
    db: &DatabaseConnection,
    table: &str,
    column: &str,
    declaration: &str,
) -> Result<()> {
    let rows = db
        .query_all_raw(Statement::from_string(
            DatabaseBackend::Sqlite,
            format!("PRAGMA table_info({table});"),
        ))
        .await
        .with_context(|| format!("inspecting {table}"))?;
    for row in &rows {
        let name: String = row
            .try_get_by_index(1)
            .with_context(|| format!("reading a {table} column name"))?;
        if name == column {
            return Ok(());
        }
    }
    db.execute_unprepared(&format!(
        "ALTER TABLE {table} ADD COLUMN {column} {declaration};"
    ))
    .await
    .with_context(|| format!("adding {table}.{column}"))?;
    Ok(())
}

/// Load the device identity (generating it on first boot) and all paired
/// instances and their values.
pub async fn load_snapshot(
    db: &DatabaseConnection,
    model: &str,
    version: &str,
) -> Result<Snapshot> {
    let existing = entity::device::Entity::find().one(db).await?;
    let mut identity;
    let keys;
    match existing {
        Some(row) => {
            identity = DeviceIdentity {
                id: row.id.clone(),
                name: row.name.clone(),
                model: row.model.clone(),
                version: row.version.clone(),
            };
            // A row migrated from the token era has empty key columns; mint the
            // device's Noise identity once and persist it.
            keys = if row.noise_private.is_empty() || row.noise_public.is_empty() {
                let keys = DeviceKeys::generate()?;
                entity::device::Entity::update_many()
                    .col_expr(
                        entity::device::Column::NoisePrivate,
                        Expr::value(keys.private_hex()),
                    )
                    .col_expr(
                        entity::device::Column::NoisePublic,
                        Expr::value(keys.public_hex()),
                    )
                    .filter(entity::device::Column::Id.eq(row.id.clone()))
                    .exec(db)
                    .await
                    .context("persisting generated device keys")?;
                keys
            } else {
                DeviceKeys {
                    private: crate::noise::from_hex(&row.noise_private)
                        .context("decoding the device private key")?,
                    public: crate::noise::from_hex(&row.noise_public)
                        .context("decoding the device public key")?,
                }
            };
        }
        None => {
            identity = DeviceIdentity::generate(model, version)?;
            keys = DeviceKeys::generate()?;
            entity::device::ActiveModel {
                id: Set(identity.id.clone()),
                name: Set(identity.name.clone()),
                model: Set(identity.model.clone()),
                version: Set(identity.version.clone()),
                noise_private: Set(keys.private_hex()),
                noise_public: Set(keys.public_hex()),
            }
            .insert(db)
            .await
            .context("inserting device identity")?;
        }
    }

    // Devices first seen before names were generated carry `screensight-<hex>`;
    // re-baptise them once so older installs get a friendly name too.
    if crate::names::is_legacy(&identity.name) {
        identity.name = crate::names::generate().context("re-baptising device")?;
        entity::device::Entity::update_many()
            .col_expr(
                entity::device::Column::Name,
                Expr::value(identity.name.clone()),
            )
            .filter(entity::device::Column::Id.eq(identity.id.clone()))
            .exec(db)
            .await
            .context("updating device name")?;
    }

    let rows = entity::instance::Entity::find().all(db).await?;
    let mut instances = Vec::with_capacity(rows.len());
    let mut selected = None;
    for row in rows {
        if row.selected {
            selected = Some(row.ha_id.clone());
        }
        instances.push(PairedInstance {
            ha_id: row.ha_id,
            ha_name: row.ha_name,
            ha_static_key: row.ha_static_key,
            last_ip: row.last_ip,
            paired_at_unix: u64::try_from(row.paired_at).unwrap_or(0),
        });
    }

    let kv_rows = entity::kv::Entity::find().all(db).await?;
    let mut values: HashMap<String, BTreeMap<String, String>> = HashMap::new();
    for row in kv_rows {
        values
            .entry(row.instance_id)
            .or_default()
            .insert(row.key, row.value);
    }

    Ok(Snapshot {
        identity,
        keys,
        instances,
        selected,
        values,
    })
}

/// Spawn the background writer that drains [`PersistCommand`]s into `db`.
pub fn spawn_writer(
    db: DatabaseConnection,
    mut rx: UnboundedReceiver<PersistCommand>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(command) = rx.recv().await {
            if let Err(err) = apply(&db, command).await {
                log::warn!("persistence write failed: {err:#}");
            }
        }
    })
}

async fn apply(db: &DatabaseConnection, command: PersistCommand) -> Result<()> {
    match command {
        PersistCommand::UpsertInstance(instance) => upsert_instance(db, &instance).await,
        PersistCommand::RemoveInstance(ha_id) => {
            entity::kv::Entity::delete_many()
                .filter(entity::kv::Column::InstanceId.eq(ha_id.clone()))
                .exec(db)
                .await?;
            entity::instance::Entity::delete_by_id(ha_id)
                .exec(db)
                .await?;
            Ok(())
        }
        PersistCommand::SetSelected(ha_id) => set_selected(db, ha_id.as_deref()).await,
        PersistCommand::SetValue { ha_id, key, value } => {
            entity::kv::Entity::insert(entity::kv::ActiveModel {
                instance_id: Set(ha_id),
                key: Set(key),
                value: Set(value),
            })
            .on_conflict(
                OnConflict::columns([entity::kv::Column::InstanceId, entity::kv::Column::Key])
                    .update_column(entity::kv::Column::Value)
                    .to_owned(),
            )
            .exec(db)
            .await?;
            Ok(())
        }
        PersistCommand::ReplaceValues { ha_id, values } => replace_values(db, &ha_id, values).await,
    }
}

async fn upsert_instance(db: &DatabaseConnection, instance: &PairedInstance) -> Result<()> {
    entity::instance::Entity::insert(entity::instance::ActiveModel {
        ha_id: Set(instance.ha_id.clone()),
        ha_name: Set(instance.ha_name.clone()),
        ha_static_key: Set(instance.ha_static_key.clone()),
        last_ip: Set(instance.last_ip.clone()),
        paired_at: Set(i64::try_from(instance.paired_at_unix).unwrap_or(i64::MAX)),
        selected: Set(false),
    })
    .on_conflict(
        OnConflict::column(entity::instance::Column::HaId)
            .update_columns([
                entity::instance::Column::HaName,
                entity::instance::Column::HaStaticKey,
                entity::instance::Column::LastIp,
                entity::instance::Column::PairedAt,
            ])
            .to_owned(),
    )
    .exec(db)
    .await?;
    Ok(())
}

async fn set_selected(db: &DatabaseConnection, ha_id: Option<&str>) -> Result<()> {
    entity::instance::Entity::update_many()
        .col_expr(entity::instance::Column::Selected, Expr::value(false))
        .exec(db)
        .await?;
    if let Some(ha_id) = ha_id {
        entity::instance::Entity::update_many()
            .col_expr(entity::instance::Column::Selected, Expr::value(true))
            .filter(entity::instance::Column::HaId.eq(ha_id))
            .exec(db)
            .await?;
    }
    Ok(())
}

async fn replace_values(
    db: &DatabaseConnection,
    ha_id: &str,
    values: BTreeMap<String, String>,
) -> Result<()> {
    entity::kv::Entity::delete_many()
        .filter(entity::kv::Column::InstanceId.eq(ha_id))
        .exec(db)
        .await?;
    if values.is_empty() {
        return Ok(());
    }
    let models = values
        .into_iter()
        .map(|(key, value)| entity::kv::ActiveModel {
            instance_id: Set(ha_id.to_owned()),
            key: Set(key),
            value: Set(value),
        })
        .collect::<Vec<_>>();
    entity::kv::Entity::insert_many(models)
        .exec_without_returning(db)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::new_instance;

    #[tokio::test]
    async fn identity_and_pairing_round_trip_through_sqlite() {
        let db = connect("sqlite::memory:").await.unwrap();

        let first = load_snapshot(&db, "Screensight Studio", "0.1.0")
            .await
            .unwrap();
        assert!(first.instances.is_empty());
        assert_eq!(first.identity.model, "Screensight Studio");
        assert_eq!(first.keys.public.len(), 32);

        // Reload keeps the same identity and static keys.
        let second = load_snapshot(&db, "ignored", "ignored").await.unwrap();
        assert_eq!(first.identity.id, second.identity.id);
        assert_eq!(first.keys.public, second.keys.public);
        assert_eq!(first.keys.private, second.keys.private);

        // Pair an instance, select it and set a value.
        let instance = new_instance("ha1", "Home", "ab".repeat(32), None);
        apply(&db, PersistCommand::UpsertInstance(instance))
            .await
            .unwrap();
        apply(&db, PersistCommand::SetSelected(Some("ha1".to_owned())))
            .await
            .unwrap();
        apply(
            &db,
            PersistCommand::SetValue {
                ha_id: "ha1".to_owned(),
                key: "text".to_owned(),
                value: "Hi".to_owned(),
            },
        )
        .await
        .unwrap();

        let loaded = load_snapshot(&db, "x", "y").await.unwrap();
        assert_eq!(loaded.instances.len(), 1);
        assert_eq!(loaded.selected.as_deref(), Some("ha1"));
        assert_eq!(loaded.values["ha1"]["text"], "Hi");

        // Removing the instance drops it and its values.
        apply(&db, PersistCommand::RemoveInstance("ha1".to_owned()))
            .await
            .unwrap();
        let loaded = load_snapshot(&db, "x", "y").await.unwrap();
        assert!(loaded.instances.is_empty());
        assert!(!loaded.values.contains_key("ha1"));
    }

    #[tokio::test]
    async fn file_database_uses_wal_and_foreign_keys() {
        use sea_orm::{DatabaseBackend, Statement};

        let dir = std::env::temp_dir().join(format!(
            "screensight-db-wal-{}-{}",
            std::process::id(),
            crate::store::now_unix()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let db = open(&crate::store::db_path(&dir)).await.unwrap();

        let mode: String = db
            .query_one_raw(Statement::from_string(
                DatabaseBackend::Sqlite,
                "PRAGMA journal_mode;".to_owned(),
            ))
            .await
            .unwrap()
            .unwrap()
            .try_get_by_index(0)
            .unwrap();
        assert_eq!(mode.to_ascii_lowercase(), "wal");

        let foreign_keys: i64 = db
            .query_one_raw(Statement::from_string(
                DatabaseBackend::Sqlite,
                "PRAGMA foreign_keys;".to_owned(),
            ))
            .await
            .unwrap()
            .unwrap()
            .try_get_by_index(0)
            .unwrap();
        assert_eq!(foreign_keys, 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn legacy_device_names_are_replaced() {
        let db = connect("sqlite::memory:").await.unwrap();
        entity::device::ActiveModel {
            id: Set("abc123".to_owned()),
            name: Set("screensight-deadbeef".to_owned()),
            model: Set("Screensight Studio".to_owned()),
            version: Set("0.1.0".to_owned()),
            noise_private: Set(String::new()),
            noise_public: Set(String::new()),
        }
        .insert(&db)
        .await
        .unwrap();

        let snapshot = load_snapshot(&db, "ignored", "ignored").await.unwrap();
        assert_eq!(snapshot.identity.id, "abc123");
        assert!(!crate::names::is_legacy(&snapshot.identity.name));
        // A migrated row with empty key columns is given a static keypair.
        assert_eq!(snapshot.keys.public.len(), 32);

        // The new name is persisted, not regenerated on every read.
        let reloaded = load_snapshot(&db, "ignored", "ignored").await.unwrap();
        assert_eq!(reloaded.identity.name, snapshot.identity.name);
        assert_eq!(reloaded.keys.public, snapshot.keys.public);
    }

    #[tokio::test]
    async fn legacy_token_schema_is_reset_and_invalidated() {
        let db = connect("sqlite::memory:").await.unwrap();

        // Recreate the pre-Noise schema, including a "paired" token instance.
        db.execute_unprepared("DROP TABLE IF EXISTS instances")
            .await
            .unwrap();
        db.execute_unprepared(
            "CREATE TABLE instances (
                ha_id TEXT PRIMARY KEY NOT NULL,
                ha_name TEXT NOT NULL,
                token TEXT NOT NULL,
                last_ip TEXT,
                paired_at INTEGER NOT NULL,
                selected INTEGER NOT NULL DEFAULT 0
            )",
        )
        .await
        .unwrap();
        db.execute_unprepared("INSERT INTO instances VALUES ('ha1', 'Home', 'tok', NULL, 0, 1)")
            .await
            .unwrap();
        db.execute_unprepared("PRAGMA user_version = 1;")
            .await
            .unwrap();

        // Re-running migration upgrades (and invalidates) the old rows.
        super::migrate(&db).await.unwrap();
        let loaded = load_snapshot(&db, "m", "v").await.unwrap();
        assert!(
            loaded.instances.is_empty(),
            "token pairings must be dropped"
        );
        assert_eq!(loaded.keys.public.len(), 32);
    }
}
