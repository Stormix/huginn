use crate::{ChatMessage, ChatMessageMetadata, ServiceError};
use serde::{Deserialize, Serialize};

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct KickWSChatMessageEvent {
    pub event: String,
    pub data: String,
}

#[derive(Debug, Deserialize)]
pub struct KickWSChatMessageData {
    pub id: String,
    pub chatroom_id: i64,
    pub content: String,
    pub created_at: String,
    pub sender: KickWSSender,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct KickWSSender {
    pub id: i64,
    pub username: String,
    pub slug: String,
    pub identity: KickWSIdentity,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct KickWSIdentity {
    pub color: String,
    pub badges: Vec<KickWSBadge>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct KickWSBadge {
    #[serde(rename = "type")]
    pub badge_type: String,
    pub text: String,
}

pub fn parse_chat_message(streamer: &str, message_text: &str) -> Result<ChatMessage, ServiceError> {
    let event: KickWSChatMessageEvent = serde_json::from_str(message_text)
        .map_err(|e| ServiceError::ParseError(format!("Outer parse error: {}", e)))?;

    let message_data: KickWSChatMessageData = serde_json::from_str(&event.data)
        .map_err(|e| ServiceError::ParseError(format!("Inner parse error: {}", e)))?;

    let timestamp = chrono::DateTime::parse_from_rfc3339(&message_data.created_at)
        .map_err(|e| ServiceError::ParseError(e.to_string()))?
        .with_timezone(&chrono::Utc);

    let metadata: ChatMessageMetadata = serde_json::from_value(serde_json::json!({
        "badges": message_data.sender.identity.badges,
        "color": message_data.sender.identity.color,
        "user_id": message_data.sender.id,
        "message_id": message_data.id,
        "chatroom_id": message_data.chatroom_id,
    }))?;

    Ok(ChatMessage {
        streamer: streamer.to_string(),
        username: message_data.sender.username,
        message: message_data.content,
        timestamp,
        metadata: Some(metadata),
    })
}
