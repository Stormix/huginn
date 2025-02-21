pub use sea_orm_migration::prelude::*;

mod m20240320_000001_create_chat_messages_table;
mod m20240321_000001_create_streamers_table;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20240320_000001_create_chat_messages_table::Migration),
            Box::new(m20240321_000001_create_streamers_table::Migration),
        ]
    }
}
