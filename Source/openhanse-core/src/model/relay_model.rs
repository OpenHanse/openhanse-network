use crate::model::message_model::ChatMessageEnvelopeModel;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RelayAttachRequestModel {
    pub relay_session_id: Uuid,
    pub peer_id: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RelayAttachResponseModel {
    pub accepted: bool,
    pub relay_session_id: Uuid,
    pub peer_id: String,
    pub counterpart_peer_id: String,
    pub expires_at_unix_ms: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RelaySendRequestModel {
    pub relay_session_id: Uuid,
    pub peer_id: String,
    pub payload: ChatMessageEnvelopeModel,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RelaySendResponseModel {
    pub accepted: bool,
    pub relay_session_id: Uuid,
    pub recipient_peer_id: String,
    pub queued_messages: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RelayPollRequestModel {
    pub peer_id: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RelayPollResponseModel {
    pub messages: Vec<RelayMessageEnvelopeModel>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RelayMessageEnvelopeModel {
    pub relay_session_id: Uuid,
    pub source_peer_id: String,
    pub target_peer_id: String,
    pub queued_at_unix_ms: u64,
    pub payload: ChatMessageEnvelopeModel,
}
