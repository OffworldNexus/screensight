//! SQLite persistence through SeaORM.
//!
//! Three tables: the single `device` row (identity), `instances` (paired Home
//! Assistant instances and which one is selected) and `kv` (per-instance
//! dashboard values). The connection is owned by a background writer task; the
//! runtime only sends it [`PersistCommand`]s, so writes never block the panel.
//!
//! SeaORM creates the initial schema directly from the entities on first boot.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use sea_orm::entity::prelude::*;
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection, Schema, Set};
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
            /// X25519 static private key, hex-encoded.
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
    create_schema(&db).await?;
    Ok(db)
}

/// Create missing tables without duplicating the entity definitions or touching
/// persisted identities, pairings and dashboard values on subsequent boots.
async fn create_schema(db: &DatabaseConnection) -> Result<()> {
    let backend = db.get_database_backend();
    let schema = Schema::new(backend);
    for mut table in [
        schema.create_table_from_entity(entity::device::Entity),
        schema.create_table_from_entity(entity::instance::Entity),
        schema.create_table_from_entity(entity::kv::Entity),
    ] {
        db.execute_raw(backend.build(table.if_not_exists()))
            .await
            .context("creating database schema")?;
    }
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
    let identity;
    let keys;
    match existing {
        Some(row) => {
            identity = DeviceIdentity {
                id: row.id.clone(),
                name: row.name.clone(),
                model: row.model.clone(),
                version: row.version.clone(),
            };
            keys = DeviceKeys {
                private: crate::noise::from_hex(&row.noise_private)
                    .context("decoding the device private key")?,
                public: crate::noise::from_hex(&row.noise_public)
                    .context("decoding the device public key")?,
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

        // Schema initialization is idempotent and never clears paired state.
        create_schema(&db).await.unwrap();
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
}
