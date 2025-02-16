use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;
 
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(ChatMessages::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(ChatMessages::Id).integer().not_null().auto_increment().primary_key())
                    .col(ColumnDef::new(ChatMessages::Streamer).string().not_null())
                    .col(ColumnDef::new(ChatMessages::Username).string().not_null())
                    .col(ColumnDef::new(ChatMessages::Message).text().not_null())
                    .col(ColumnDef::new(ChatMessages::Timestamp).timestamp_with_time_zone().not_null())
                    .col(ColumnDef::new(ChatMessages::Metadata).json())
                    .col(ColumnDef::new(ChatMessages::CreatedAt).timestamp_with_time_zone().not_null())
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(ChatMessages::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum ChatMessages {
    Table,
    Id,
    Streamer,
    Username,
    Message,
    Timestamp,
    Metadata,
    CreatedAt,
}