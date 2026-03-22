use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChatMessageEnvelopeModel {
    pub from_peer_id: String,
    pub to_peer_id: String,
    pub message: String,
    pub sent_at_unix_ms: u64,
}
