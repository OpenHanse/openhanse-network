use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReachabilityScopeModel {
    Loopback,
    Lan,
    Public,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReachabilitySourceModel {
    LocalBind,
    LocalDetection,
    HubObserved,
    DiscoveryProbe,
    Manual,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TransportProtocolEnum {
    Relay,
    DirectTcp,
    // Reserved for the later UDP hole-punching delivery path.
    DirectUdp,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReachabilityConfidenceModel {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DirectReachabilityModeModel {
    RelayOnly,
    LocalOnly,
    UnknownExternal,
    PublicDirect,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NatBehaviorModel {
    Unknown,
    Predictable,
    Symmetric,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ReachabilityAddressModel {
    pub base_url: String,
    pub scope: ReachabilityScopeModel,
    pub source: ReachabilitySourceModel,
    pub transport_protocol: TransportProtocolEnum,
    pub confidence: ReachabilityConfidenceModel,
    pub address_hint: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct PeerReachabilityModel {
    pub mode: DirectReachabilityModeModel,
    pub nat_behavior: NatBehaviorModel,
    pub message_endpoint: Option<String>,
    pub bind_address: Option<ReachabilityAddressModel>,
    pub advertised_addresses: Vec<ReachabilityAddressModel>,
    pub observed_addresses: Vec<ReachabilityAddressModel>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RegisterPeerRequestModel {
    pub peer_id: String,
    pub device_key: String,
    #[serde(default)]
    pub display_name: Option<String>,
    pub reachability: PeerReachabilityModel,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HeartbeatRequestModel {
    pub peer_id: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PeerRecordModel {
    pub peer_id: String,
    pub device_key: String,
    pub display_name: Option<String>,
    pub reachability: PeerReachabilityModel,
    pub registered_at_unix_ms: u64,
    pub expires_at_unix_ms: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RegisterPeerResponseModel {
    pub lease_seconds: u64,
    pub peer: PeerRecordModel,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PeerLookupResponseModel {
    pub peer: PeerRecordModel,
}
