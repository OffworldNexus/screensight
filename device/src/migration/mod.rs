//! Ordered schema history, recorded by SeaORM in `seaql_migrations`.
//!
//! Append new migrations here; keep applied migrations unchanged so an upgrade
//! runs the same schema changes regardless of the current entity definitions.

use sea_orm_migration::prelude::*;

mod m000001_create_tables;

/// Applies each numbered migration in order when the device opens its database.
pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(m000001_create_tables::Migration)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn initial_migration_is_recorded_and_reversible() {
        let db = sea_orm::Database::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        let applied = Migrator::get_applied_migrations(&db).await.unwrap();
        assert_eq!(applied.len(), 1);
        assert_eq!(applied[0].name(), "m000001_create_tables");
        let manager = SchemaManager::new(&db);
        for table in ["device", "instances", "kv", "seaql_migrations"] {
            assert!(manager.has_table(table).await.unwrap());
        }

        Migrator::up(&db, None).await.unwrap();
        assert_eq!(
            Migrator::get_applied_migrations(&db).await.unwrap().len(),
            1
        );

        Migrator::down(&db, Some(1)).await.unwrap();
        assert!(Migrator::get_applied_migrations(&db)
            .await
            .unwrap()
            .is_empty());
        for table in ["device", "instances", "kv"] {
            assert!(!manager.has_table(table).await.unwrap());
        }

        Migrator::up(&db, None).await.unwrap();
        assert_eq!(
            Migrator::get_applied_migrations(&db).await.unwrap().len(),
            1
        );
        for table in ["device", "instances", "kv"] {
            assert!(manager.has_table(table).await.unwrap());
        }
    }
}
