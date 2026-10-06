//! Initial Noise schema. Definitions are frozen here rather than generated
//! from runtime entities, so later entity changes cannot rewrite schema history.

use sea_orm_migration::prelude::*;

/// Establishes device identity, paired peers and per-peer dashboard values.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Device::Table)
                    .col(ColumnDef::new(Device::Id).string().not_null().primary_key())
                    .col(ColumnDef::new(Device::Name).string().not_null())
                    .col(ColumnDef::new(Device::Model).string().not_null())
                    .col(ColumnDef::new(Device::Version).string().not_null())
                    .col(ColumnDef::new(Device::NoisePrivate).string().not_null())
                    .col(ColumnDef::new(Device::NoisePublic).string().not_null())
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(Instances::Table)
                    .col(
                        ColumnDef::new(Instances::HaId)
                            .string()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(Instances::HaName).string().not_null())
                    .col(ColumnDef::new(Instances::HaStaticKey).string().not_null())
                    .col(ColumnDef::new(Instances::LastIp).string())
                    .col(ColumnDef::new(Instances::PairedAt).big_integer().not_null())
                    .col(
                        ColumnDef::new(Instances::Selected)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(Kv::Table)
                    .col(ColumnDef::new(Kv::InstanceId).string().not_null())
                    .col(ColumnDef::new(Kv::Key).string().not_null())
                    .col(ColumnDef::new(Kv::Value).string().not_null())
                    .primary_key(Index::create().col(Kv::InstanceId).col(Kv::Key))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Kv::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Instances::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Device::Table).to_owned())
            .await
    }
}

/// Fixed identifiers for the initial device identity schema.
#[derive(DeriveIden)]
enum Device {
    Table,
    Id,
    Name,
    Model,
    Version,
    NoisePrivate,
    NoisePublic,
}

/// Fixed identifiers for paired Home Assistant instances.
#[derive(DeriveIden)]
enum Instances {
    Table,
    HaId,
    HaName,
    HaStaticKey,
    LastIp,
    PairedAt,
    Selected,
}

/// Fixed identifiers for the composite-key dashboard values table.
#[derive(DeriveIden)]
enum Kv {
    Table,
    InstanceId,
    Key,
    Value,
}
