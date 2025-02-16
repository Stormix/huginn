use sea_orm::entity::prelude::*;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "chat_messages")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub streamer: String,
    pub username: String,
    pub message: String,
    pub timestamp: DateTime<Utc>,
    #[sea_orm(column_type = "Json")]
    pub metadata: Option<Json>,
    pub created_at: DateTime<Utc>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {} 