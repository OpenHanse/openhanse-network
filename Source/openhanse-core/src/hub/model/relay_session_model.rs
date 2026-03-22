use crate::model::relay_model::RelayMessageEnvelopeModel;
use std::collections::VecDeque;

#[derive(Clone)]
pub struct RelaySessionModel {
    pub source_peer_id: String,
    pub target_peer_id: String,
    pub expires_at: std::time::Instant,
    pub source_attached: bool,
    pub target_attached: bool,
    pub pending_for_source: VecDeque<RelayMessageEnvelopeModel>,
    pub pending_for_target: VecDeque<RelayMessageEnvelopeModel>,
}
