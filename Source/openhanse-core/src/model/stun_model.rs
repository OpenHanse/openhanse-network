use serde::{Deserialize, Serialize};

use crate::model::peer_model::TransportProtocolEnum;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DiscoveryProbeRequestModel {
    pub transaction_id: String,
    pub peer_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DiscoveryProbeResponseModel {
    pub transaction_id: String,
    pub observed_addr: String,
    pub transport_protocol: TransportProtocolEnum,
}
