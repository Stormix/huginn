use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Streamers::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Streamers::Username)
                            .string()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(Streamers::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        // Insert default streamers
        let insert_sql = r#"
        INSERT INTO streamers (username, created_at)
        VALUES 
            ('ilyaselmaliki', CURRENT_TIMESTAMP),
            ('111fox', CURRENT_TIMESTAMP),
            ('rainman-fps', CURRENT_TIMESTAMP),
            ('ahmedsabiri', CURRENT_TIMESTAMP),
            ('chaos333gg', CURRENT_TIMESTAMP),
            ('boushaq', CURRENT_TIMESTAMP)
        ON CONFLICT DO NOTHING;
        "#;

        manager
            .get_connection()
            .execute_unprepared(insert_sql)
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Streamers::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum Streamers {
    Table,
    Username,
    CreatedAt,
}
