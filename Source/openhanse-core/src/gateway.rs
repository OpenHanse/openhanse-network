use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::post,
};
use crate::model::{
    connect_model::ConnectDecisionModel,
    message_model::ChatMessageEnvelopeModel,
    peer_model::{
        DirectReachabilityModeModel, HeartbeatRequestModel, NatBehaviorModel,
        PeerLookupResponseModel, PeerReachabilityModel, ReachabilityAddressModel,
        ReachabilityConfidenceModel, ReachabilityScopeModel, ReachabilitySourceModel,
        RegisterPeerRequestModel,
        RegisterPeerResponseModel, TransportProtocolEnum,
    },
    relay_model::{
        RelayAttachRequestModel, RelayAttachResponseModel, RelayMessageEnvelopeModel,
        RelayPollRequestModel, RelayPollResponseModel, RelaySendRequestModel,
        RelaySendResponseModel,
    },
    stun_model::{DiscoveryProbeRequestModel, DiscoveryProbeResponseModel},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::hash_map::DefaultHasher,
    fs::{self, OpenOptions},
    hash::{Hash, Hasher},
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    net::TcpListener,
    sync::{Mutex, RwLock, broadcast, watch},
    task::JoinHandle,
};

const DEFAULT_MESSAGE_ENDPOINT: &str = "/message";
const DEFAULT_HEARTBEAT_INTERVAL_SECS: u64 = 10;
const DEFAULT_RELAY_POLL_INTERVAL_MS: u64 = 1000;
const DEFAULT_DISCOVERY_UDP_PORT: u16 = 3478;
const MAX_EVENTS: usize = 256;
const DELIVERY_MODE_DIRECT_TCP: &str = "direct_tcp";
const DELIVERY_MODE_RELAY: &str = "relay";

#[derive(Debug, Clone)]
struct UdpDiscoveryOutcome {
    nat_behavior: NatBehaviorModel,
    observed_addresses: Vec<ReachabilityAddressModel>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GatewayProfile {
    pub peer_id: String,
    pub device_key: String,
    pub display_name: Option<String>,
    pub reachability: PeerReachabilityModel,
}

impl GatewayProfile {
    pub fn register_request(&self) -> RegisterPeerRequestModel {
        RegisterPeerRequestModel {
            peer_id: self.peer_id.clone(),
            device_key: self.device_key.clone(),
            display_name: self.display_name.clone(),
            reachability: self.reachability.clone(),
        }
    }

    pub fn heartbeat_request(&self) -> HeartbeatRequestModel {
        HeartbeatRequestModel {
            peer_id: self.peer_id.clone(),
        }
    }

    pub fn connect_request(
        &self,
        target_peer_id: impl Into<String>,
        prefer_direct: bool,
    ) -> crate::model::connect_model::ConnectRequestModel {
        crate::model::connect_model::ConnectRequestModel {
            source_peer_id: self.peer_id.clone(),
            target_peer_id: target_peer_id.into(),
            prefer_direct,
        }
    }

    pub fn outbound_message(
        &self,
        target_peer_id: impl Into<String>,
        message: impl Into<String>,
        sent_at_unix_ms: u64,
    ) -> ChatMessageEnvelopeModel {
        ChatMessageEnvelopeModel {
            from_peer_id: self.peer_id.clone(),
            to_peer_id: target_peer_id.into(),
            message: message.into(),
            sent_at_unix_ms,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommunicationModeEnum {
    Auto,
    Direct,
    RelayOnly,
}

impl CommunicationModeEnum {
    pub fn prefer_direct(self) -> bool {
        matches!(self, Self::Auto | Self::Direct)
    }

    pub fn supports_direct_advertisement(self) -> bool {
        !matches!(self, Self::RelayOnly)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Direct => "direct",
            Self::RelayOnly => "relay",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GatewayRuntimeConfig {
    pub peer_id: String,
    pub device_key: String,
    pub display_name: Option<String>,
    pub target_peer_id: String,
    pub server_base_url: String,
    pub direct_bind_host: String,
    pub direct_bind_port: u16,
    #[serde(default = "default_communication_mode")]
    pub communication_mode: CommunicationModeEnum,
    pub heartbeat_interval_secs: u64,
    pub storage_dir: PathBuf,
}

impl GatewayRuntimeConfig {
    pub fn from_profile(
        profile: GatewayProfile,
        target_peer_id: String,
        server_base_url: String,
        direct_bind_host: String,
        direct_bind_port: u16,
        heartbeat_interval_secs: u64,
        storage_dir: PathBuf,
    ) -> Self {
        Self {
            peer_id: profile.peer_id,
            device_key: profile.device_key,
            display_name: profile.display_name,
            target_peer_id,
            server_base_url,
            direct_bind_host,
            direct_bind_port,
            communication_mode: if profile.reachability.mode == DirectReachabilityModeModel::RelayOnly {
                CommunicationModeEnum::RelayOnly
            } else {
                CommunicationModeEnum::Auto
            },
            heartbeat_interval_secs,
            storage_dir,
        }
    }

    pub fn normalized(mut self) -> Self {
        self.server_base_url = normalize_server_base_url(&self.server_base_url);
        if self.heartbeat_interval_secs == 0 {
            self.heartbeat_interval_secs = DEFAULT_HEARTBEAT_INTERVAL_SECS;
        }
        if self.communication_mode == CommunicationModeEnum::Auto
            && !should_advertise_direct_endpoint(&self.direct_bind_host)
        {
            self.communication_mode = CommunicationModeEnum::RelayOnly;
        }
        self
    }

    pub fn profile(&self) -> GatewayProfile {
        GatewayProfile {
            peer_id: self.peer_id.clone(),
            device_key: self.device_key.clone(),
            display_name: self.display_name.clone(),
            reachability: peer_reachability_for(
                &self.direct_bind_host,
                self.direct_bind_port,
                self.communication_mode.supports_direct_advertisement(),
                DEFAULT_MESSAGE_ENDPOINT,
                NatBehaviorModel::Unknown,
                Vec::new(),
            ),
        }
    }

    pub fn inbox_file(&self) -> PathBuf {
        self.storage_dir.join("inbox.jsonl")
    }

    pub fn events_file(&self) -> PathBuf {
        self.storage_dir.join("events.jsonl")
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GatewayRuntimeInfoModel {
    pub peer_id: String,
    pub target_peer_id: String,
    pub server_base_url: String,
    pub direct_base_url: String,
    pub message_endpoint: String,
    pub storage_dir: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GatewayRuntimeStatusModel {
    pub peer_id: String,
    pub display_name: Option<String>,
    pub target_peer_id: String,
    pub communication_mode: String,
    pub server_base_url: String,
    pub direct_base_url: String,
    pub message_endpoint: String,
    pub heartbeat_interval_secs: u64,
    pub heartbeat_state: String,
    pub last_registered_at_unix_ms: Option<u64>,
    pub last_heartbeat_at_unix_ms: Option<u64>,
    pub last_error: Option<String>,
    pub last_delivery_mode: Option<String>,
    pub last_delivery_summary: Option<String>,
    pub inbox_count: usize,
    pub event_count: usize,
    pub direct_sent_count: usize,
    pub relay_sent_count: usize,
    pub direct_received_count: usize,
    pub relay_received_count: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct InboxEntryModel {
    pub received_at_unix_ms: u64,
    pub peer_id: String,
    pub payload: ChatMessageEnvelopeModel,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UiEventModel {
    pub id: u64,
    pub kind: String,
    pub message: String,
    pub created_at_unix_ms: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SendMessageResponseModel {
    pub accepted: bool,
    pub delivery_mode: String,
    pub communication_mode: String,
    pub target_url: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct InboxListResponseModel {
    pub inbox: Vec<InboxEntryModel>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EventsResponseModel {
    pub events: Vec<UiEventModel>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ErrorResponseModel {
    pub error: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SendMessageRequestModel {
    pub message: String,
}

#[derive(Clone)]
pub struct GatewayRuntimeHandle {
    shared: Arc<GatewayRuntimeShared>,
}

struct GatewayRuntimeShared {
    config: GatewayRuntimeConfig,
    profile: GatewayProfile,
    direct_base_url: String,
    state: RwLock<GatewayRuntimeState>,
    hub_client: HubClient,
    shutdown_tx: watch::Sender<bool>,
    event_tx: broadcast::Sender<UiEventModel>,
    tasks: Mutex<Vec<JoinHandle<()>>>,
}

struct GatewayRuntimeState {
    last_registered_at_unix_ms: Option<u64>,
    last_heartbeat_at_unix_ms: Option<u64>,
    lease_seconds: Option<u64>,
    last_error: Option<String>,
    last_delivery_mode: Option<String>,
    last_delivery_summary: Option<String>,
    inbox: Vec<InboxEntryModel>,
    events: Vec<UiEventModel>,
    next_event_id: u64,
    direct_sent_count: usize,
    relay_sent_count: usize,
    direct_received_count: usize,
    relay_received_count: usize,
}

#[derive(Clone)]
struct DirectTcpMessageAppState {
    runtime: GatewayRuntimeHandle,
}

impl GatewayRuntimeHandle {
    pub async fn start(config: GatewayRuntimeConfig) -> Result<Self, String> {
        let config = config.normalized();
        fs::create_dir_all(&config.storage_dir).map_err(|error| error.to_string())?;

        let direct_listener =
            bind_direct_listener(&config.direct_bind_host, config.direct_bind_port).await?;
        let direct_address = direct_listener
            .local_addr()
            .map_err(|error| format!("failed to inspect direct listener: {error}"))?;
        let udp_discovery_result = if config.communication_mode.supports_direct_advertisement() {
            discover_udp_reachability(
                &config.server_base_url,
                &config.peer_id,
                &config.direct_bind_host,
                direct_address.port(),
            )
            .await
            .map(Some)
        } else {
            Ok(None)
        };
        let udp_discovery = udp_discovery_result
            .as_ref()
            .ok()
            .and_then(|observed| observed.clone())
            .unwrap_or_else(|| UdpDiscoveryOutcome {
                nat_behavior: NatBehaviorModel::Unknown,
                observed_addresses: Vec::new(),
            });

        let profile = GatewayProfile {
            reachability: peer_reachability_for(
                &config.direct_bind_host,
                direct_address.port(),
                config.communication_mode.supports_direct_advertisement(),
                DEFAULT_MESSAGE_ENDPOINT,
                udp_discovery.nat_behavior,
                udp_discovery.observed_addresses.clone(),
            ),
            ..config.profile()
        };
        let inbox = load_json_lines::<InboxEntryModel>(&config.inbox_file())?;
        let events = load_json_lines::<UiEventModel>(&config.events_file())?;
        let next_event_id = events.last().map(|event| event.id + 1).unwrap_or(1);
        let hub_client = HubClient::new(config.server_base_url.clone())?;
        let (shutdown_tx, _) = watch::channel(false);
        let (event_tx, _) = broadcast::channel(128);

        let shared = Arc::new(GatewayRuntimeShared {
            config: config.clone(),
            profile: profile.clone(),
            direct_base_url: format!("http://{}:{}", config.direct_bind_host, direct_address.port()),
            state: RwLock::new(GatewayRuntimeState {
                last_registered_at_unix_ms: None,
                last_heartbeat_at_unix_ms: None,
                lease_seconds: None,
                last_error: None,
                last_delivery_mode: None,
                last_delivery_summary: None,
                inbox,
                events,
                next_event_id,
                direct_sent_count: 0,
                relay_sent_count: 0,
                direct_received_count: 0,
                relay_received_count: 0,
            }),
            hub_client,
            shutdown_tx,
            event_tx,
            tasks: Mutex::new(Vec::new()),
        });
        let handle = Self { shared };

        handle
            .record_event(
                "runtime_started",
                format!(
                    "Runtime started for {} targeting {}.",
                    handle.shared.profile.peer_id, handle.shared.config.target_peer_id
                ),
            )
            .await?;
        match udp_discovery_result {
            Ok(Some(outcome)) if !outcome.observed_addresses.is_empty() => {
                handle
                    .record_event(
                        "udp_discovery_succeeded",
                        format!(
                            "Observed {} UDP mapping(s); NAT classified as {}.",
                            outcome.observed_addresses.len(),
                            nat_behavior_label(outcome.nat_behavior),
                        ),
                    )
                    .await?;
            }
            Ok(_) => {}
            Err(error) => {
                handle
                    .record_event(
                        "udp_discovery_failed",
                        format!("UDP discovery probe failed: {error}"),
                    )
                    .await?;
            }
        }

        handle.spawn_direct_tcp_receiver(direct_listener).await;
        handle.register().await?;
        handle.spawn_heartbeat_loop().await;
        handle.spawn_relay_poll_loop().await;

        Ok(handle)
    }

    pub async fn stop(&self) -> Result<(), String> {
        let _ = self.shared.shutdown_tx.send(true);
        let mut tasks = self.shared.tasks.lock().await;
        for task in tasks.drain(..) {
            let _ = task.await;
        }
        self.record_event(
            "runtime_stopped",
            format!("Runtime stopped for {}.", self.shared.profile.peer_id),
        )
        .await?;
        Ok(())
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<UiEventModel> {
        self.shared.event_tx.subscribe()
    }

    pub async fn status(&self) -> GatewayRuntimeStatusModel {
        let state = self.shared.state.read().await;
        GatewayRuntimeStatusModel {
            peer_id: self.shared.profile.peer_id.clone(),
            display_name: self.shared.profile.display_name.clone(),
            target_peer_id: self.shared.config.target_peer_id.clone(),
            communication_mode: self.shared.config.communication_mode.as_str().to_string(),
            server_base_url: self.shared.config.server_base_url.clone(),
            direct_base_url: self.shared.direct_base_url.clone(),
            message_endpoint: self
                .shared
                .profile
                .reachability
                .message_endpoint
                .clone()
                .unwrap_or_else(|| DEFAULT_MESSAGE_ENDPOINT.to_string()),
            heartbeat_interval_secs: self.shared.config.heartbeat_interval_secs,
            heartbeat_state: heartbeat_state(&state),
            last_registered_at_unix_ms: state.last_registered_at_unix_ms,
            last_heartbeat_at_unix_ms: state.last_heartbeat_at_unix_ms,
            last_error: state.last_error.clone(),
            last_delivery_mode: state.last_delivery_mode.clone(),
            last_delivery_summary: state.last_delivery_summary.clone(),
            inbox_count: state.inbox.len(),
            event_count: state.events.len(),
            direct_sent_count: state.direct_sent_count,
            relay_sent_count: state.relay_sent_count,
            direct_received_count: state.direct_received_count,
            relay_received_count: state.relay_received_count,
        }
    }

    pub fn info(&self) -> GatewayRuntimeInfoModel {
        GatewayRuntimeInfoModel {
            peer_id: self.shared.profile.peer_id.clone(),
            target_peer_id: self.shared.config.target_peer_id.clone(),
            server_base_url: self.shared.config.server_base_url.clone(),
            direct_base_url: self.shared.direct_base_url.clone(),
            message_endpoint: self
                .shared
                .profile
                .reachability
                .message_endpoint
                .clone()
                .unwrap_or_else(|| DEFAULT_MESSAGE_ENDPOINT.to_string()),
            storage_dir: self.shared.config.storage_dir.to_string_lossy().to_string(),
        }
    }

    pub async fn list_inbox(&self) -> Vec<InboxEntryModel> {
        self.shared.state.read().await.inbox.clone()
    }

    pub async fn events_since(&self, since_event_id: Option<u64>) -> Vec<UiEventModel> {
        let state = self.shared.state.read().await;
        match since_event_id {
            Some(since_event_id) => state
                .events
                .iter()
                .filter(|event| event.id > since_event_id)
                .cloned()
                .collect(),
            None => state.events.clone(),
        }
    }

    pub async fn lookup_target(&self) -> Result<PeerLookupResponseModel, String> {
        self.record_event(
            "lookup_started",
            format!("Looking up {}.", self.shared.config.target_peer_id),
        )
        .await?;
        match self
            .shared
            .hub_client
            .lookup(&self.shared.config.target_peer_id)
            .await
        {
            Ok(response) => {
                self.record_event(
                    "lookup_succeeded",
                    format!("{} is online.", response.peer.peer_id),
                )
                .await?;
                Ok(response)
            }
            Err(error) => {
                self.record_error(&error).await?;
                Err(error)
            }
        }
    }

    pub async fn connect_target(&self) -> Result<ConnectDecisionModel, String> {
        self.connect_target_with_mode(self.shared.config.communication_mode)
            .await
    }

    async fn connect_target_with_mode(
        &self,
        communication_mode: CommunicationModeEnum,
    ) -> Result<ConnectDecisionModel, String> {
        self.record_event(
            "connect_started",
            format!(
                "Requesting {} connect decision for {}.",
                communication_mode.as_str(),
                self.shared.config.target_peer_id
            ),
        )
        .await?;
        match self
            .shared
            .hub_client
            .connect(
                &self.shared.profile,
                &self.shared.config.target_peer_id,
                communication_mode,
            )
            .await
        {
            Ok(decision) => {
                let message = match &decision {
                    ConnectDecisionModel::Direct { direct } => format!(
                        "Connect decision: direct to {} via {} ({})",
                        direct.peer_id,
                        direct
                            .reachability_candidates
                            .first()
                            .map(|candidate| candidate.base_url.clone())
                            .unwrap_or_else(|| "<missing>".to_string()),
                        direct.decision_reason,
                    ),
                    ConnectDecisionModel::Relay { relay } => format!(
                        "Connect decision: relay {} for {} -> {} ({})",
                        relay.relay_session_id,
                        relay.source_peer_id,
                        relay.target_peer_id,
                        relay.decision_reason
                    ),
                };
                self.record_event("connect_succeeded", message).await?;
                Ok(decision)
            }
            Err(error) => {
                self.record_error(&error).await?;
                Err(error)
            }
        }
    }

    pub async fn send_message(&self, message: impl Into<String>) -> Result<SendMessageResponseModel, String> {
        let message = message.into();
        let communication_mode = self.shared.config.communication_mode;
        let decision = self.connect_target_with_mode(communication_mode).await?;
        match decision {
            ConnectDecisionModel::Direct { direct } => {
                let direct_address = direct
                    .reachability_candidates
                    .first()
                    .map(|candidate| candidate.base_url.clone())
                    .ok_or_else(|| format!("target peer '{}' has no direct address", direct.peer_id))?;
                let target_url = join_url(
                    &direct_address,
                    direct
                        .message_endpoint
                        .as_deref()
                        .unwrap_or(DEFAULT_MESSAGE_ENDPOINT),
                );
                let payload = self.shared.profile.outbound_message(
                    self.shared.config.target_peer_id.clone(),
                    message.clone(),
                    current_unix_ms(),
                );
                match self
                    .shared
                    .hub_client
                    .post_direct_message(&target_url, &payload)
                    .await
                {
                    Ok(_) => {
                        self.remember_delivery(
                            "direct",
                            format!(
                            "Sent direct_tcp message to {} via {}.",
                                self.shared.config.target_peer_id, target_url
                            ),
                        )
                        .await;
                        self.record_event(
                            "message_sent",
                            format!("Sent message to {}.", self.shared.config.target_peer_id),
                        )
                        .await?;
                        Ok(SendMessageResponseModel {
                            accepted: true,
                            delivery_mode: DELIVERY_MODE_DIRECT_TCP.to_string(),
                            communication_mode: communication_mode.as_str().to_string(),
                            target_url,
                        })
                    }
                    Err(error) => {
                        if communication_mode == CommunicationModeEnum::Direct {
                            self.record_error(&error).await?;
                            return Err(format!(
                                "direct delivery failed in forced direct mode: {error}"
                            ));
                        }
                        self.record_event(
                            "direct_delivery_failed",
                            format!(
                                "Direct TCP delivery to {} failed, retrying via relay: {}",
                                self.shared.config.target_peer_id, error
                            ),
                        )
                        .await?;
                        let relay_decision = self
                            .connect_target_with_mode(CommunicationModeEnum::RelayOnly)
                            .await?;
                        match relay_decision {
                            ConnectDecisionModel::Relay { relay } => {
                                self.send_message_via_relay(relay, payload).await
                            }
                            ConnectDecisionModel::Direct { .. } => {
                                self.record_error(&error).await?;
                                Err(error)
                            }
                        }
                    }
                }
            }
            ConnectDecisionModel::Relay { relay } => {
                if communication_mode == CommunicationModeEnum::Direct {
                    let error = format!(
                        "forced direct mode could not find a credible direct path: {}",
                        relay.decision_reason
                    );
                    self.record_error(&error).await?;
                    return Err(error);
                }
                let payload = self.shared.profile.outbound_message(
                    self.shared.config.target_peer_id.clone(),
                    message,
                    current_unix_ms(),
                );
                self.send_message_via_relay(relay, payload).await
            }
        }
    }

    pub async fn register(&self) -> Result<RegisterPeerResponseModel, String> {
        match self.shared.hub_client.register(&self.shared.profile).await {
            Ok(response) => {
                let mut state = self.shared.state.write().await;
                state.last_registered_at_unix_ms = Some(current_unix_ms());
                state.last_heartbeat_at_unix_ms = state.last_registered_at_unix_ms;
                state.lease_seconds = Some(response.lease_seconds);
                state.last_error = None;
                drop(state);
                self.record_event(
                    "register_succeeded",
                    format!("Registered {}.", self.shared.profile.peer_id),
                )
                .await?;
                Ok(response)
            }
            Err(error) => {
                self.record_error(&error).await?;
                Err(error)
            }
        }
    }

    async fn heartbeat(&self) -> Result<RegisterPeerResponseModel, String> {
        match self.shared.hub_client.heartbeat(&self.shared.profile).await {
            Ok(response) => {
                let mut state = self.shared.state.write().await;
                state.last_heartbeat_at_unix_ms = Some(current_unix_ms());
                state.lease_seconds = Some(response.lease_seconds);
                state.last_error = None;
                Ok(response)
            }
            Err(error) => {
                self.record_error(&error).await?;
                Err(error)
            }
        }
    }

    async fn receive_direct_tcp_message(
        &self,
        payload: ChatMessageEnvelopeModel,
    ) -> Result<AcceptedResponse, String> {
        self.persist_received_message(payload, DELIVERY_MODE_DIRECT_TCP).await?;

        Ok(AcceptedResponse {
            status: "accepted".to_string(),
            peer_id: self.shared.profile.peer_id.clone(),
        })
    }

    async fn receive_relay_message(
        &self,
        envelope: RelayMessageEnvelopeModel,
    ) -> Result<(), String> {
        self.persist_received_message(envelope.payload, DELIVERY_MODE_RELAY).await?;
        self.record_event(
            "relay_message_received",
            format!(
                "Received relay message on session {}.",
                envelope.relay_session_id
            ),
        )
        .await
    }

    async fn persist_received_message(
        &self,
        payload: ChatMessageEnvelopeModel,
        delivery_mode: &str,
    ) -> Result<(), String> {
        let entry = InboxEntryModel {
            received_at_unix_ms: current_unix_ms(),
            peer_id: self.shared.profile.peer_id.clone(),
            payload: payload.clone(),
        };
        append_json_line(&self.shared.config.inbox_file(), &entry).map_err(|error| error.to_string())?;
        {
            let mut state = self.shared.state.write().await;
            state.inbox.push(entry);
            state.last_delivery_mode = Some(delivery_mode.to_string());
            state.last_delivery_summary = Some(format!(
                "Received {} message from {}.",
                delivery_mode, payload.from_peer_id
            ));
            match delivery_mode {
                DELIVERY_MODE_DIRECT_TCP => state.direct_received_count += 1,
                DELIVERY_MODE_RELAY => state.relay_received_count += 1,
                _ => {}
            }
        }
        self.record_event(
            "message_received",
            format!(
                "Received {} message from {}: {}",
                delivery_mode, payload.from_peer_id, payload.message
            ),
        )
        .await
    }

    async fn spawn_direct_tcp_receiver(&self, listener: TcpListener) {
        let app = Router::new()
            .route(DEFAULT_MESSAGE_ENDPOINT, post(receive_direct_tcp_message_endpoint))
            .with_state(DirectTcpMessageAppState {
                runtime: self.clone(),
            });
        let mut shutdown_rx = self.shared.shutdown_tx.subscribe();
        let task = tokio::spawn(async move {
            let server = axum::serve(listener, app).with_graceful_shutdown(async move {
                let _ = shutdown_rx.changed().await;
            });
            let _ = server.await;
        });
        self.shared.tasks.lock().await.push(task);
    }

    async fn spawn_heartbeat_loop(&self) {
        let runtime = self.clone();
        let mut shutdown_rx = self.shared.shutdown_tx.subscribe();
        let interval_secs = self.shared.config.heartbeat_interval_secs;
        let task = tokio::spawn(async move {
            let duration = Duration::from_secs(interval_secs);
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(duration) => {
                        if let Err(error) = runtime.heartbeat().await {
                            let _ = runtime.record_error(&error).await;
                        }
                    }
                    _ = shutdown_rx.changed() => break,
                }
            }
        });
        self.shared.tasks.lock().await.push(task);
    }

    async fn spawn_relay_poll_loop(&self) {
        let runtime = self.clone();
        let mut shutdown_rx = self.shared.shutdown_tx.subscribe();
        let task = tokio::spawn(async move {
            let duration = Duration::from_millis(DEFAULT_RELAY_POLL_INTERVAL_MS);
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(duration) => {
                        match runtime.shared.hub_client.poll_relay_messages(&runtime.shared.profile.peer_id).await {
                            Ok(response) => {
                                for envelope in response.messages {
                                    if let Err(error) = runtime.receive_relay_message(envelope).await {
                                        let _ = runtime.record_error(&error).await;
                                    }
                                }
                            }
                            Err(error) => {
                                let _ = runtime.record_error(&error).await;
                            }
                        }
                    }
                    _ = shutdown_rx.changed() => break,
                }
            }
        });
        self.shared.tasks.lock().await.push(task);
    }

    async fn send_message_via_relay(
        &self,
        relay: crate::model::connect_model::RelayConnectionInfoModel,
        payload: ChatMessageEnvelopeModel,
    ) -> Result<SendMessageResponseModel, String> {
        self.shared
            .hub_client
            .attach_relay(relay.relay_session_id, &self.shared.profile.peer_id)
            .await?;
        self.shared
            .hub_client
            .send_relay_message(relay.relay_session_id, &self.shared.profile.peer_id, &payload)
            .await?;
        self.remember_delivery(
            DELIVERY_MODE_RELAY,
            format!(
                "Sent relay message to {} using session {}.",
                payload.to_peer_id, relay.relay_session_id
            ),
        )
        .await;
        self.record_event(
            "message_sent",
            format!(
                "Sent relay message to {} using session {}.",
                payload.to_peer_id, relay.relay_session_id
            ),
        )
        .await?;
        Ok(SendMessageResponseModel {
            accepted: true,
            delivery_mode: DELIVERY_MODE_RELAY.to_string(),
            communication_mode: self.shared.config.communication_mode.as_str().to_string(),
            target_url: join_url(&self.shared.config.server_base_url, "/v1/relay/send"),
        })
    }

    async fn record_error(&self, error: &str) -> Result<(), String> {
        {
            let mut state = self.shared.state.write().await;
            state.last_error = Some(error.to_string());
        }
        self.record_event("error", error.to_string()).await
    }

    async fn remember_delivery(&self, delivery_mode: &str, summary: String) {
        let mut state = self.shared.state.write().await;
        state.last_delivery_mode = Some(delivery_mode.to_string());
        state.last_delivery_summary = Some(summary);
        match delivery_mode {
            DELIVERY_MODE_DIRECT_TCP => state.direct_sent_count += 1,
            DELIVERY_MODE_RELAY => state.relay_sent_count += 1,
            _ => {}
        }
    }

    async fn record_event(&self, kind: impl Into<String>, message: impl Into<String>) -> Result<(), String> {
        let event = {
            let mut state = self.shared.state.write().await;
            let event = UiEventModel {
                id: state.next_event_id,
                kind: kind.into(),
                message: message.into(),
                created_at_unix_ms: current_unix_ms(),
            };
            state.next_event_id += 1;
            state.events.push(event.clone());
            if state.events.len() > MAX_EVENTS {
                let overflow = state.events.len() - MAX_EVENTS;
                state.events.drain(0..overflow);
            }
            event
        };
        append_json_line(&self.shared.config.events_file(), &event).map_err(|error| error.to_string())?;
        let _ = self.shared.event_tx.send(event);
        Ok(())
    }
}

async fn bind_direct_listener(bind_host: &str, bind_port: u16) -> Result<TcpListener, String> {
    let mut attempts = Vec::new();
    for candidate in direct_bind_candidates(bind_host, bind_port) {
        match TcpListener::bind((candidate.as_str(), bind_port)).await {
            Ok(listener) => return Ok(listener),
            Err(error) => attempts.push(format!("{candidate}:{bind_port}: {error}")),
        }
    }

    Err(format!(
        "failed to bind direct receiver on {bind_host}:{bind_port} ({})",
        attempts.join("; ")
    ))
}

fn direct_bind_candidates(bind_host: &str, bind_port: u16) -> Vec<String> {
    let mut candidates = vec![bind_host.to_string()];
    if bind_port == 0 && !is_loopback_host(bind_host) && bind_host != "0.0.0.0" {
        candidates.push("0.0.0.0".to_string());
    }
    candidates
}

fn is_loopback_host(host: &str) -> bool {
    matches!(host, "127.0.0.1" | "localhost" | "::1")
}

pub fn should_advertise_direct_endpoint(bind_host: &str) -> bool {
    !is_loopback_host(bind_host)
}

fn peer_reachability_for(
    bind_host: &str,
    bind_port: u16,
    supports_direct: bool,
    message_endpoint: &str,
    nat_behavior: NatBehaviorModel,
    observed_addresses: Vec<ReachabilityAddressModel>,
) -> PeerReachabilityModel {
    let bind_base_url = format!("http://{}:{}", bind_host, bind_port);
    let bind_scope = reachability_scope_for_host(bind_host);

    if !supports_direct {
        return PeerReachabilityModel {
            mode: DirectReachabilityModeModel::RelayOnly,
            nat_behavior: NatBehaviorModel::Unknown,
            message_endpoint: Some(message_endpoint.to_string()),
            bind_address: Some(ReachabilityAddressModel {
                base_url: bind_base_url,
                scope: bind_scope,
                source: ReachabilitySourceModel::LocalBind,
                transport_protocol: TransportProtocolEnum::DirectTcp,
                confidence: ReachabilityConfidenceModel::High,
                address_hint: bind_host.to_string(),
            }),
            advertised_addresses: Vec::new(),
            observed_addresses,
        };
    }

    let bind_address = ReachabilityAddressModel {
        base_url: bind_base_url.clone(),
        scope: bind_scope,
        source: ReachabilitySourceModel::LocalBind,
        transport_protocol: TransportProtocolEnum::DirectTcp,
        confidence: ReachabilityConfidenceModel::High,
        address_hint: bind_host.to_string(),
    };
    let advertised_address = ReachabilityAddressModel {
        base_url: bind_base_url,
        scope: bind_scope,
        source: ReachabilitySourceModel::LocalDetection,
        transport_protocol: TransportProtocolEnum::DirectTcp,
        confidence: match bind_scope {
            ReachabilityScopeModel::Public => ReachabilityConfidenceModel::High,
            ReachabilityScopeModel::Lan => ReachabilityConfidenceModel::Medium,
            ReachabilityScopeModel::Loopback => ReachabilityConfidenceModel::Low,
        },
        address_hint: bind_host.to_string(),
    };

    PeerReachabilityModel {
        mode: match bind_scope {
            ReachabilityScopeModel::Public => DirectReachabilityModeModel::PublicDirect,
            ReachabilityScopeModel::Lan => DirectReachabilityModeModel::UnknownExternal,
            ReachabilityScopeModel::Loopback => DirectReachabilityModeModel::LocalOnly,
        },
        nat_behavior,
        message_endpoint: Some(message_endpoint.to_string()),
        bind_address: Some(bind_address),
        advertised_addresses: vec![advertised_address],
        observed_addresses,
    }
}

fn reachability_scope_for_host(host: &str) -> ReachabilityScopeModel {
    if is_loopback_host(host) {
        return ReachabilityScopeModel::Loopback;
    }

    if host.eq_ignore_ascii_case("0.0.0.0") {
        return ReachabilityScopeModel::Lan;
    }

    match host.parse::<std::net::IpAddr>() {
        Ok(ip) if ip.is_loopback() => ReachabilityScopeModel::Loopback,
        Ok(std::net::IpAddr::V4(ip)) if ip.is_private() || ip.is_link_local() => {
            ReachabilityScopeModel::Lan
        }
        Ok(std::net::IpAddr::V6(ip)) if ip.is_unique_local() || ip.is_unicast_link_local() => {
            ReachabilityScopeModel::Lan
        }
        Ok(_) => ReachabilityScopeModel::Public,
        Err(_) => ReachabilityScopeModel::Public,
    }
}

async fn discover_udp_reachability(
    server_base_url: &str,
    peer_id: &str,
    bind_host: &str,
    _bind_port: u16,
) -> Result<UdpDiscoveryOutcome, String> {
    let discovery_addrs = derive_discovery_udp_addrs(server_base_url).await?;
    let socket = tokio::net::UdpSocket::bind((bind_host, 0))
        .await
        .map_err(|error| format!("failed to bind UDP discovery socket: {error}"))?;
    let first = probe_udp_observed_address(&socket, discovery_addrs[0], peer_id).await?;
    let second = probe_udp_observed_address(&socket, discovery_addrs[1], peer_id).await?;
    let nat_behavior = nat_behavior_from_probes(first, second);

    let mut observed_addresses = Vec::new();
    for observed_addr in [first, second] {
        let candidate = ReachabilityAddressModel {
            base_url: format!("udp://{}", observed_addr),
            scope: reachability_scope_for_ip(observed_addr.ip()),
            source: ReachabilitySourceModel::DiscoveryProbe,
            transport_protocol: TransportProtocolEnum::DirectUdp,
            confidence: match nat_behavior {
                NatBehaviorModel::Predictable => ReachabilityConfidenceModel::High,
                NatBehaviorModel::Symmetric => ReachabilityConfidenceModel::Medium,
                NatBehaviorModel::Unknown => ReachabilityConfidenceModel::Low,
            },
            address_hint: observed_addr.ip().to_string(),
        };
        if !observed_addresses
            .iter()
            .any(|existing: &ReachabilityAddressModel| existing.base_url == candidate.base_url)
        {
            observed_addresses.push(candidate);
        }
    }

    Ok(UdpDiscoveryOutcome {
        nat_behavior,
        observed_addresses,
    })
}

async fn probe_udp_observed_address(
    socket: &tokio::net::UdpSocket,
    discovery_addr: SocketAddr,
    peer_id: &str,
) -> Result<SocketAddr, String> {
    let request = DiscoveryProbeRequestModel {
        transaction_id: uuid::Uuid::new_v4().to_string(),
        peer_id: Some(peer_id.to_string()),
    };
    let request_bytes = serde_json::to_vec(&request).map_err(|error| error.to_string())?;
    socket
        .send_to(&request_bytes, discovery_addr)
        .await
        .map_err(|error| format!("failed to send UDP discovery probe: {error}"))?;

    let mut buffer = [0_u8; 1024];
    let (len, _) = tokio::time::timeout(
        Duration::from_millis(1500),
        socket.recv_from(&mut buffer),
    )
    .await
    .map_err(|_| "UDP discovery probe timed out".to_string())?
    .map_err(|error| format!("failed to receive UDP discovery response: {error}"))?;

    let response: DiscoveryProbeResponseModel =
        serde_json::from_slice(&buffer[..len]).map_err(|error| error.to_string())?;
    if response.transaction_id != request.transaction_id {
        return Err("UDP discovery response transaction mismatch".to_string());
    }
    let observed_addr = response
        .observed_addr
        .parse::<SocketAddr>()
        .map_err(|error| format!("invalid observed UDP address: {error}"))?;

    Ok(observed_addr)
}

fn nat_behavior_label(nat_behavior: NatBehaviorModel) -> &'static str {
    match nat_behavior {
        NatBehaviorModel::Unknown => "unknown",
        NatBehaviorModel::Predictable => "predictable",
        NatBehaviorModel::Symmetric => "symmetric",
    }
}

fn nat_behavior_from_probes(
    first: SocketAddr,
    second: SocketAddr,
) -> NatBehaviorModel {
    if first.port() == second.port() {
        NatBehaviorModel::Predictable
    } else {
        NatBehaviorModel::Symmetric
    }
}

async fn derive_discovery_udp_addrs(server_base_url: &str) -> Result<[SocketAddr; 2], String> {
    let url = reqwest::Url::parse(server_base_url).map_err(|error| error.to_string())?;
    let host = url
        .host_str()
        .ok_or_else(|| "server base URL is missing a host".to_string())?;
    let primary = resolve_discovery_udp_addr(host, DEFAULT_DISCOVERY_UDP_PORT).await?;
    let secondary = resolve_discovery_udp_addr(host, DEFAULT_DISCOVERY_UDP_PORT + 1).await?;
    Ok([primary, secondary])
}

async fn resolve_discovery_udp_addr(host: &str, port: u16) -> Result<SocketAddr, String> {
    tokio::net::lookup_host((host, port))
        .await
        .map_err(|error| format!("failed to resolve UDP discovery host {host}:{port}: {error}"))?
        .next()
        .ok_or_else(|| format!("UDP discovery host {host}:{port} resolved to no addresses"))
}

fn reachability_scope_for_ip(ip: std::net::IpAddr) -> ReachabilityScopeModel {
    match ip {
        std::net::IpAddr::V4(ip) if ip.is_loopback() => ReachabilityScopeModel::Loopback,
        std::net::IpAddr::V4(ip) if ip.is_private() || ip.is_link_local() => {
            ReachabilityScopeModel::Lan
        }
        std::net::IpAddr::V6(ip) if ip.is_loopback() => ReachabilityScopeModel::Loopback,
        std::net::IpAddr::V6(ip) if ip.is_unique_local() || ip.is_unicast_link_local() => {
            ReachabilityScopeModel::Lan
        }
        _ => ReachabilityScopeModel::Public,
    }
}

async fn receive_direct_tcp_message_endpoint(
    State(state): State<DirectTcpMessageAppState>,
    Json(payload): Json<ChatMessageEnvelopeModel>,
) -> Result<Json<AcceptedResponse>, (StatusCode, Json<ErrorResponseModel>)> {
    state
        .runtime
        .receive_direct_tcp_message(payload)
        .await
        .map(Json)
        .map_err(internal_string_error)
}

#[derive(Deserialize, Serialize)]
struct AcceptedResponse {
    status: String,
    peer_id: String,
}

#[derive(Clone)]
struct HubClient {
    http_client: reqwest::Client,
    server_base_url: String,
}

impl HubClient {
    fn new(server_base_url: String) -> Result<Self, String> {
        let http_client = reqwest::Client::builder()
            .build()
            .map_err(|error| error.to_string())?;

        Ok(Self {
            http_client,
            server_base_url,
        })
    }

    async fn register(
        &self,
        profile: &GatewayProfile,
    ) -> Result<RegisterPeerResponseModel, String> {
        self.post_json("/v1/peers/register", &profile.register_request())
            .await
    }

    async fn heartbeat(
        &self,
        profile: &GatewayProfile,
    ) -> Result<RegisterPeerResponseModel, String> {
        self.post_json("/v1/peers/heartbeat", &profile.heartbeat_request())
            .await
    }

    async fn lookup(&self, target_peer_id: &str) -> Result<PeerLookupResponseModel, String> {
        let url = format!("{}/v1/peers/{}", self.server_base_url, target_peer_id);
        let response = self
            .http_client
            .get(url)
            .send()
            .await
            .map_err(|error| error.to_string())?;
        decode_json_response(response).await
    }

    async fn connect(
        &self,
        profile: &GatewayProfile,
        target_peer_id: &str,
        communication_mode: CommunicationModeEnum,
    ) -> Result<ConnectDecisionModel, String> {
        self.post_json(
            "/v1/connect",
            &profile.connect_request(
                target_peer_id.to_string(),
                communication_mode.prefer_direct(),
            ),
        )
        .await
    }

    async fn post_direct_message(
        &self,
        target_url: &str,
        payload: &ChatMessageEnvelopeModel,
    ) -> Result<AcceptedResponse, String> {
        let response = self
            .http_client
            .post(target_url)
            .json(payload)
            .send()
            .await
            .map_err(|error| error.to_string())?;
        decode_json_response(response).await
    }

    async fn attach_relay(
        &self,
        relay_session_id: uuid::Uuid,
        peer_id: &str,
    ) -> Result<RelayAttachResponseModel, String> {
        self.post_json(
            "/v1/relay/attach",
            &RelayAttachRequestModel {
                relay_session_id,
                peer_id: peer_id.to_string(),
            },
        )
        .await
    }

    async fn send_relay_message(
        &self,
        relay_session_id: uuid::Uuid,
        peer_id: &str,
        payload: &ChatMessageEnvelopeModel,
    ) -> Result<RelaySendResponseModel, String> {
        self.post_json(
            "/v1/relay/send",
            &RelaySendRequestModel {
                relay_session_id,
                peer_id: peer_id.to_string(),
                payload: payload.clone(),
            },
        )
        .await
    }

    async fn poll_relay_messages(
        &self,
        peer_id: &str,
    ) -> Result<RelayPollResponseModel, String> {
        self.post_json(
            "/v1/relay/poll",
            &RelayPollRequestModel {
                peer_id: peer_id.to_string(),
            },
        )
        .await
    }

    async fn post_json<Request: Serialize, Response: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        payload: &Request,
    ) -> Result<Response, String> {
        let url = format!("{}{}", self.server_base_url, path);
        let response = self
            .http_client
            .post(url)
            .json(payload)
            .send()
            .await
            .map_err(|error| error.to_string())?;
        decode_json_response(response).await
    }
}

async fn decode_json_response<Response: serde::de::DeserializeOwned>(
    response: reqwest::Response,
) -> Result<Response, String> {
    let status = response.status();
    let body = response.text().await.map_err(|error| error.to_string())?;

    if !status.is_success() {
        if body.is_empty() {
            return Err(format!("HTTP {}", status));
        }
        return Err(format!("HTTP {}: {}", status, body));
    }

    serde_json::from_str(&body).map_err(|error| format!("invalid response body: {error}"))
}

fn heartbeat_state(state: &GatewayRuntimeState) -> String {
    if state.last_error.is_some() {
        return "error".to_string();
    }
    if state.last_heartbeat_at_unix_ms.is_some() {
        return "online".to_string();
    }
    if state.last_registered_at_unix_ms.is_some() {
        return "registered".to_string();
    }
    "starting".to_string()
}

fn default_communication_mode() -> CommunicationModeEnum {
    CommunicationModeEnum::Auto
}

fn append_json_line<T: Serialize>(path: &Path, value: &T) -> Result<(), std::io::Error> {
    ensure_parent_dir(path).map_err(std::io::Error::other)?;
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    use std::io::Write;

    serde_json::to_writer(&mut file, value)?;
    file.write_all(b"\n")?;
    Ok(())
}

fn load_json_lines<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Vec<T>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let contents = fs::read_to_string(path).map_err(|error| error.to_string())?;
    contents
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).map_err(|error| error.to_string()))
        .collect()
}

fn ensure_parent_dir(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn internal_string_error(error: String) -> (StatusCode, Json<ErrorResponseModel>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponseModel { error }),
    )
}

pub fn normalize_server_base_url(value: &str) -> String {
    value.trim_end_matches('/').to_string()
}

pub fn join_url(base: &str, suffix: &str) -> String {
    if suffix.starts_with("http://") || suffix.starts_with("https://") {
        return suffix.to_string();
    }

    format!(
        "{}/{}",
        base.trim_end_matches('/'),
        suffix.trim_start_matches('/')
    )
}

pub fn default_port_for_peer(peer_id: &str) -> u16 {
    match peer_id {
        "gateway-a" => 17441,
        "gateway-b" => 17442,
        _ => {
            let mut hasher = DefaultHasher::new();
            peer_id.hash(&mut hasher);
            20000 + (hasher.finish() % 20000) as u16
        }
    }
}

pub fn display_name_for_peer(peer_id: &str) -> String {
    let mut chars = peer_id.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => "OpenHanse Peer".to_string(),
    }
}

pub fn current_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub fn default_runtime_storage_dir(root: &Path, peer_id: &str) -> PathBuf {
    root.join("peers").join(peer_id)
}

pub fn socket_addr_port(address: &str) -> Option<u16> {
    address.parse::<SocketAddr>().ok().map(|socket| socket.port())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Json, Router, extract::Path, routing::get};
    use crate::model::{
        connect_model::{ConnectDecisionModel, DirectConnectionInfoModel},
        peer_model::{
            NatBehaviorModel, PeerLookupResponseModel, PeerRecordModel,
            ReachabilityAddressModel, ReachabilityConfidenceModel, ReachabilityScopeModel,
            ReachabilitySourceModel, TransportProtocolEnum,
        },
        relay_model::{
            RelayAttachRequestModel, RelayAttachResponseModel, RelayMessageEnvelopeModel,
            RelayPollRequestModel, RelayPollResponseModel, RelaySendRequestModel,
            RelaySendResponseModel,
        },
        stun_model::{DiscoveryProbeRequestModel, DiscoveryProbeResponseModel},
    };
    use std::{collections::{HashMap, VecDeque}, sync::Arc};
    use tokio::sync::RwLock as TokioRwLock;
    use uuid::Uuid;

    #[test]
    fn builds_register_request_from_profile() {
        let profile = gateway_profile("gateway-a", 7443);
        let request = profile.register_request();

        assert_eq!(request.peer_id, "gateway-a");
        assert_eq!(request.reachability.message_endpoint.as_deref(), Some("/message"));
    }

    #[test]
    fn communication_mode_controls_direct_preference() {
        assert!(CommunicationModeEnum::Auto.prefer_direct());
        assert!(CommunicationModeEnum::Direct.prefer_direct());
        assert!(!CommunicationModeEnum::RelayOnly.prefer_direct());
    }

    #[test]
    fn normalizes_join_url() {
        assert_eq!(join_url("http://127.0.0.1:8080/", "/message"), "http://127.0.0.1:8080/message");
    }

    #[tokio::test]
    async fn direct_message_flow_works_between_two_runtimes() {
        let hub = TestHub::start().await;
        let runtime_a = GatewayRuntimeHandle::start(runtime_config(
            "gateway-a",
            "gateway-b",
            &hub.base_url,
            unique_temp_dir("gateway-a"),
        ))
        .await
        .expect("start runtime a");
        let runtime_b = GatewayRuntimeHandle::start(runtime_config(
            "gateway-b",
            "gateway-a",
            &hub.base_url,
            unique_temp_dir("gateway-b"),
        ))
        .await
        .expect("start runtime b");

        let target_url = join_url(&runtime_b.info().direct_base_url, DEFAULT_MESSAGE_ENDPOINT);
        let payload =
            runtime_a
                .shared
                .profile
                .outbound_message("gateway-b", "hello from runtime a", current_unix_ms());
        let response = reqwest::Client::new()
            .post(target_url)
            .json(&payload)
            .send()
            .await
            .expect("post direct message");
        assert!(response.status().is_success());

        tokio::time::sleep(Duration::from_millis(200)).await;

        let inbox = runtime_b.list_inbox().await;
        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox[0].payload.message, "hello from runtime a");

        runtime_a.stop().await.expect("stop runtime a");
        runtime_b.stop().await.expect("stop runtime b");
    }

    #[tokio::test]
    async fn relay_message_flow_works_between_two_runtimes() {
        let hub = TestHub::start().await;
        let mut runtime_a_config = runtime_config(
            "gateway-a",
            "gateway-b",
            &hub.base_url,
            unique_temp_dir("gateway-a-relay"),
        );
        runtime_a_config.communication_mode = CommunicationModeEnum::RelayOnly;

        let mut runtime_b_config = runtime_config(
            "gateway-b",
            "gateway-a",
            &hub.base_url,
            unique_temp_dir("gateway-b-relay"),
        );
        runtime_b_config.communication_mode = CommunicationModeEnum::RelayOnly;

        let runtime_a = GatewayRuntimeHandle::start(runtime_a_config)
            .await
            .expect("start runtime a");
        let runtime_b = GatewayRuntimeHandle::start(runtime_b_config)
            .await
            .expect("start runtime b");

        let send = runtime_a
            .send_message("hello through relay")
            .await
            .expect("send relay message");
        assert_eq!(send.delivery_mode, "relay");

        tokio::time::sleep(Duration::from_millis(1200)).await;

        let inbox = runtime_b.list_inbox().await;
        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox[0].payload.message, "hello through relay");

        runtime_a.stop().await.expect("stop runtime a");
        runtime_b.stop().await.expect("stop runtime b");
    }

    #[tokio::test]
    async fn forced_direct_mode_fails_when_only_relay_is_available() {
        let hub = TestHub::start().await;
        let mut runtime_a_config = runtime_config(
            "gateway-a",
            "gateway-b",
            &hub.base_url,
            unique_temp_dir("gateway-a-direct-only"),
        );
        runtime_a_config.communication_mode = CommunicationModeEnum::Direct;

        let mut runtime_b_config = runtime_config(
            "gateway-b",
            "gateway-a",
            &hub.base_url,
            unique_temp_dir("gateway-b-direct-only"),
        );
        runtime_b_config.communication_mode = CommunicationModeEnum::RelayOnly;

        let runtime_a = GatewayRuntimeHandle::start(runtime_a_config)
            .await
            .expect("start runtime a");
        let runtime_b = GatewayRuntimeHandle::start(runtime_b_config)
            .await
            .expect("start runtime b");

        let error = runtime_a
            .send_message("hello direct only")
            .await
            .expect_err("forced direct mode should fail");
        assert!(error.contains("forced direct mode"));

        runtime_a.stop().await.expect("stop runtime a");
        runtime_b.stop().await.expect("stop runtime b");
    }

    #[tokio::test]
    async fn udp_discovery_probe_reports_observed_udp_candidate() {
        let primary_socket = tokio::net::UdpSocket::bind(("127.0.0.1", 0))
            .await
            .expect("bind primary udp discovery test server");
        let discovery_addr = primary_socket.local_addr().expect("primary discovery addr");
        let secondary_socket =
            tokio::net::UdpSocket::bind(("127.0.0.1", discovery_addr.port() + 1))
                .await
                .expect("bind secondary udp discovery test server");
        for socket in [primary_socket, secondary_socket] {
            tokio::spawn(async move {
                let mut buffer = [0_u8; 1024];
                let (len, remote_addr) = socket.recv_from(&mut buffer).await.expect("recv probe");
                let request: DiscoveryProbeRequestModel =
                    serde_json::from_slice(&buffer[..len]).expect("decode request");
                let response = DiscoveryProbeResponseModel {
                    transaction_id: request.transaction_id,
                    observed_addr: remote_addr.to_string(),
                    transport_protocol: TransportProtocolEnum::DirectUdp,
                };
                let payload = serde_json::to_vec(&response).expect("encode response");
                socket
                    .send_to(&payload, remote_addr)
                    .await
                    .expect("send response");
            });
        }

        let client_socket = tokio::net::UdpSocket::bind(("127.0.0.1", 0))
            .await
            .expect("bind client udp discovery socket");
        let first = probe_udp_observed_address(&client_socket, discovery_addr, "gateway-a")
            .await
            .expect("probe primary");
        let second = probe_udp_observed_address(
            &client_socket,
            std::net::SocketAddr::from(([127, 0, 0, 1], discovery_addr.port() + 1)),
            "gateway-a",
        )
        .await
        .expect("probe secondary");
        assert_eq!(nat_behavior_from_probes(first, second), NatBehaviorModel::Predictable);
    }

    #[tokio::test]
    async fn udp_discovery_probe_reports_symmetric_nat_when_mappings_differ() {
        let primary_socket = tokio::net::UdpSocket::bind(("127.0.0.1", 0))
            .await
            .expect("bind primary udp discovery test server");
        let discovery_addr = primary_socket.local_addr().expect("primary discovery addr");
        let secondary_socket =
            tokio::net::UdpSocket::bind(("127.0.0.1", discovery_addr.port() + 1))
                .await
                .expect("bind secondary udp discovery test server");
        tokio::spawn(async move {
            let mut buffer = [0_u8; 1024];
            let (len, remote_addr) = primary_socket
                .recv_from(&mut buffer)
                .await
                .expect("recv primary probe");
            let request: DiscoveryProbeRequestModel =
                serde_json::from_slice(&buffer[..len]).expect("decode primary request");
            let response = DiscoveryProbeResponseModel {
                transaction_id: request.transaction_id,
                observed_addr: remote_addr.to_string(),
                transport_protocol: TransportProtocolEnum::DirectUdp,
            };
            let payload = serde_json::to_vec(&response).expect("encode primary response");
            primary_socket
                .send_to(&payload, remote_addr)
                .await
                .expect("send primary response");
        });
        tokio::spawn(async move {
            let mut buffer = [0_u8; 1024];
            let (len, remote_addr) = secondary_socket
                .recv_from(&mut buffer)
                .await
                .expect("recv secondary probe");
            let request: DiscoveryProbeRequestModel =
                serde_json::from_slice(&buffer[..len]).expect("decode secondary request");
            let mut observed_addr = remote_addr;
            observed_addr.set_port(remote_addr.port() + 1000);
            let response = DiscoveryProbeResponseModel {
                transaction_id: request.transaction_id,
                observed_addr: observed_addr.to_string(),
                transport_protocol: TransportProtocolEnum::DirectUdp,
            };
            let payload = serde_json::to_vec(&response).expect("encode secondary response");
            secondary_socket
                .send_to(&payload, remote_addr)
                .await
                .expect("send secondary response");
        });

        let client_socket = tokio::net::UdpSocket::bind(("127.0.0.1", 0))
            .await
            .expect("bind client udp discovery socket");
        let first = probe_udp_observed_address(&client_socket, discovery_addr, "gateway-a")
            .await
            .expect("probe primary");
        let second = probe_udp_observed_address(
            &client_socket,
            std::net::SocketAddr::from(([127, 0, 0, 1], discovery_addr.port() + 1)),
            "gateway-a",
        )
        .await
        .expect("probe secondary");
        assert_eq!(nat_behavior_from_probes(first, second), NatBehaviorModel::Symmetric);
    }

    #[test]
    fn direct_bind_candidates_keep_loopback_hosts() {
        assert_eq!(direct_bind_candidates("127.0.0.1", 0), vec!["127.0.0.1"]);
    }

    #[test]
    fn direct_bind_candidates_fallback_to_any_for_ephemeral_lan_bind() {
        assert_eq!(
            direct_bind_candidates("192.168.1.105", 0),
            vec!["192.168.1.105", "0.0.0.0"]
        );
    }

    fn gateway_profile(peer_id: &str, port: u16) -> GatewayProfile {
        GatewayProfile {
            peer_id: peer_id.to_string(),
            device_key: format!("device-key-{peer_id}"),
            display_name: Some(display_name_for_peer(peer_id)),
            reachability: PeerReachabilityModel {
                mode: DirectReachabilityModeModel::LocalOnly,
                nat_behavior: NatBehaviorModel::Unknown,
                message_endpoint: Some("/message".to_string()),
                bind_address: Some(local_candidate(port)),
                advertised_addresses: vec![local_candidate(port)],
                observed_addresses: Vec::new(),
            },
        }
    }

    fn local_candidate(port: u16) -> ReachabilityAddressModel {
        ReachabilityAddressModel {
            base_url: format!("http://127.0.0.1:{port}"),
            scope: ReachabilityScopeModel::Loopback,
            source: ReachabilitySourceModel::LocalDetection,
            transport_protocol: TransportProtocolEnum::DirectTcp,
            confidence: ReachabilityConfidenceModel::Low,
            address_hint: "127.0.0.1".to_string(),
        }
    }

    fn runtime_config(
        peer_id: &str,
        target_peer_id: &str,
        server_base_url: &str,
        storage_dir: PathBuf,
    ) -> GatewayRuntimeConfig {
        GatewayRuntimeConfig {
            peer_id: peer_id.to_string(),
            device_key: format!("device-key-{peer_id}"),
            display_name: Some(display_name_for_peer(peer_id)),
            target_peer_id: target_peer_id.to_string(),
            server_base_url: server_base_url.to_string(),
            direct_bind_host: "127.0.0.1".to_string(),
            direct_bind_port: 0,
            communication_mode: CommunicationModeEnum::Auto,
            heartbeat_interval_secs: 60,
            storage_dir,
        }
    }

    fn unique_temp_dir(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "openhanse-gateway-test-{label}-{}",
            current_unix_ms()
        ))
    }

    #[derive(Clone)]
    struct TestHubState {
        peers: Arc<TokioRwLock<HashMap<String, PeerRecordModel>>>,
        relay_sessions: Arc<TokioRwLock<HashMap<Uuid, TestRelaySession>>>,
    }

    #[derive(Clone)]
    struct TestRelaySession {
        source_peer_id: String,
        target_peer_id: String,
        pending_for_source: VecDeque<RelayMessageEnvelopeModel>,
        pending_for_target: VecDeque<RelayMessageEnvelopeModel>,
    }

    struct TestHub {
        base_url: String,
    }

    impl TestHub {
        async fn start() -> Self {
            let state = TestHubState {
                peers: Arc::new(TokioRwLock::new(HashMap::new())),
                relay_sessions: Arc::new(TokioRwLock::new(HashMap::new())),
            };
            let app = Router::new()
                .route("/v1/peers/register", post(test_register_peer))
                .route("/v1/peers/heartbeat", post(test_heartbeat_peer))
                .route("/v1/peers/{peer_id}", get(test_lookup_peer))
                .route("/v1/connect", post(test_connect_peer))
                .route("/v1/relay/attach", post(test_relay_attach))
                .route("/v1/relay/send", post(test_relay_send))
                .route("/v1/relay/poll", post(test_relay_poll))
                .with_state(state);
            let listener = TcpListener::bind(("127.0.0.1", 0))
                .await
                .expect("bind test hub");
            let address = listener.local_addr().expect("test hub local addr");
            tokio::spawn(async move {
                let _ = axum::serve(listener, app).await;
            });
            Self {
                base_url: format!("http://127.0.0.1:{}", address.port()),
            }
        }
    }

    async fn test_register_peer(
        State(state): State<TestHubState>,
        Json(request): Json<RegisterPeerRequestModel>,
    ) -> Json<RegisterPeerResponseModel> {
        let peer = PeerRecordModel {
            peer_id: request.peer_id.clone(),
            device_key: request.device_key.clone(),
            display_name: request.display_name.clone(),
            reachability: request.reachability.clone(),
            registered_at_unix_ms: current_unix_ms(),
            expires_at_unix_ms: current_unix_ms() + 30_000,
        };
        state
            .peers
            .write()
            .await
            .insert(request.peer_id.clone(), peer.clone());

        Json(RegisterPeerResponseModel {
            lease_seconds: 30,
            peer,
        })
    }

    async fn test_heartbeat_peer(
        State(state): State<TestHubState>,
        Json(request): Json<HeartbeatRequestModel>,
    ) -> Result<Json<RegisterPeerResponseModel>, StatusCode> {
        let mut peers = state.peers.write().await;
        let Some(peer) = peers.get_mut(&request.peer_id) else {
            return Err(StatusCode::NOT_FOUND);
        };
        peer.expires_at_unix_ms = current_unix_ms() + 30_000;
        Ok(Json(RegisterPeerResponseModel {
            lease_seconds: 30,
            peer: peer.clone(),
        }))
    }

    async fn test_lookup_peer(
        State(state): State<TestHubState>,
        Path(peer_id): Path<String>,
    ) -> Result<Json<PeerLookupResponseModel>, StatusCode> {
        let peers = state.peers.read().await;
        let Some(peer) = peers.get(&peer_id) else {
            return Err(StatusCode::NOT_FOUND);
        };
        Ok(Json(PeerLookupResponseModel { peer: peer.clone() }))
    }

    async fn test_connect_peer(
        State(state): State<TestHubState>,
        Json(request): Json<crate::model::connect_model::ConnectRequestModel>,
    ) -> Result<Json<ConnectDecisionModel>, StatusCode> {
        let peers = state.peers.read().await;
        let Some(peer) = peers.get(&request.target_peer_id) else {
            return Err(StatusCode::NOT_FOUND);
        };
        if request.prefer_direct && !peer.reachability.advertised_addresses.is_empty() {
            return Ok(Json(ConnectDecisionModel::Direct {
                direct: DirectConnectionInfoModel {
                    peer_id: peer.peer_id.clone(),
                    device_key: peer.device_key.clone(),
                    display_name: peer.display_name.clone(),
                    reachability_candidates: peer.reachability.advertised_addresses.clone(),
                    message_endpoint: peer.reachability.message_endpoint.clone(),
                    decision_reason: "test hub advertised a direct candidate".to_string(),
                },
            }));
        }

        let relay_session_id = Uuid::new_v4();
        state.relay_sessions.write().await.insert(
            relay_session_id,
            TestRelaySession {
                source_peer_id: request.source_peer_id.clone(),
                target_peer_id: request.target_peer_id.clone(),
                pending_for_source: VecDeque::new(),
                pending_for_target: VecDeque::new(),
            },
        );
        Ok(Json(ConnectDecisionModel::relay(
            crate::model::connect_model::RelayConnectionInfoModel {
                relay_session_id,
                source_peer_id: request.source_peer_id,
                target_peer_id: request.target_peer_id,
                expires_at_unix_ms: current_unix_ms() + 30_000,
                decision_reason: "test hub fell back to relay".to_string(),
            },
        )))
    }

    async fn test_relay_attach(
        State(state): State<TestHubState>,
        Json(request): Json<RelayAttachRequestModel>,
    ) -> Result<Json<RelayAttachResponseModel>, StatusCode> {
        let sessions = state.relay_sessions.read().await;
        let Some(session) = sessions.get(&request.relay_session_id) else {
            return Err(StatusCode::NOT_FOUND);
        };
        let counterpart = if request.peer_id == session.source_peer_id {
            session.target_peer_id.clone()
        } else if request.peer_id == session.target_peer_id {
            session.source_peer_id.clone()
        } else {
            return Err(StatusCode::BAD_REQUEST);
        };
        Ok(Json(RelayAttachResponseModel {
            accepted: true,
            relay_session_id: request.relay_session_id,
            peer_id: request.peer_id,
            counterpart_peer_id: counterpart,
            expires_at_unix_ms: current_unix_ms() + 30_000,
        }))
    }

    async fn test_relay_send(
        State(state): State<TestHubState>,
        Json(request): Json<RelaySendRequestModel>,
    ) -> Result<Json<RelaySendResponseModel>, StatusCode> {
        let mut sessions = state.relay_sessions.write().await;
        let Some(session) = sessions.get_mut(&request.relay_session_id) else {
            return Err(StatusCode::NOT_FOUND);
        };
        let (recipient_peer_id, queue) = if request.peer_id == session.source_peer_id {
            (session.target_peer_id.clone(), &mut session.pending_for_target)
        } else if request.peer_id == session.target_peer_id {
            (session.source_peer_id.clone(), &mut session.pending_for_source)
        } else {
            return Err(StatusCode::BAD_REQUEST);
        };
        queue.push_back(RelayMessageEnvelopeModel {
            relay_session_id: request.relay_session_id,
            source_peer_id: request.payload.from_peer_id.clone(),
            target_peer_id: request.payload.to_peer_id.clone(),
            queued_at_unix_ms: current_unix_ms(),
            payload: request.payload,
        });
        Ok(Json(RelaySendResponseModel {
            accepted: true,
            relay_session_id: request.relay_session_id,
            recipient_peer_id,
            queued_messages: queue.len(),
        }))
    }

    async fn test_relay_poll(
        State(state): State<TestHubState>,
        Json(request): Json<RelayPollRequestModel>,
    ) -> Result<Json<RelayPollResponseModel>, StatusCode> {
        let mut sessions = state.relay_sessions.write().await;
        let mut messages = Vec::new();
        for session in sessions.values_mut() {
            let queue = if request.peer_id == session.source_peer_id {
                &mut session.pending_for_source
            } else if request.peer_id == session.target_peer_id {
                &mut session.pending_for_target
            } else {
                continue;
            };
            while let Some(message) = queue.pop_front() {
                messages.push(message);
            }
        }
        Ok(Json(RelayPollResponseModel { messages }))
    }
}
