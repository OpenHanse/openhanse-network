use crate::{
    hub::util::time_util::TimeUtil,
    model::peer_model::{PeerReachabilityModel, PeerRecordModel},
};
use std::time::Instant;

#[derive(Clone)]
pub struct PeerPresenceModel {
    pub peer_id: String,
    pub device_key: String,
    pub display_name: Option<String>,
    pub reachability: PeerReachabilityModel,
    pub registered_at: Instant,
    pub expires_at: Instant,
}

impl PeerPresenceModel {
    pub fn to_record_model(&self) -> PeerRecordModel {
        let now = Instant::now();
        let expires_in_ms = self
            .expires_at
            .saturating_duration_since(now)
            .as_millis()
            .min(u128::from(u64::MAX)) as u64;

        PeerRecordModel {
            peer_id: self.peer_id.clone(),
            device_key: self.device_key.clone(),
            display_name: self.display_name.clone(),
            reachability: self.reachability.clone(),
            registered_at_unix_ms: TimeUtil::unix_time_ms_from_instant(self.registered_at, now),
            expires_at_unix_ms: TimeUtil::unix_time_ms() + expires_in_ms,
        }
    }
}
