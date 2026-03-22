pub mod endpoint;
pub mod model;
pub mod util;

use crate::model::{
    peer_model::TransportProtocolEnum,
    stun_model::{DiscoveryProbeRequestModel, DiscoveryProbeResponseModel},
};
use axum::{
    Router,
    routing::{get, post},
};
use endpoint::{
    connect_endpoint::connect_peer_endpoint,
    health_endpoint::health_endpoint,
    peer_endpoint::{get_peer_endpoint, peer_heartbeat_endpoint, register_peer_endpoint},
    relay_endpoint::{relay_attach_endpoint, relay_poll_endpoint, relay_send_endpoint},
};
pub use model::app_state_model::AppStateModel;
use serde::{Deserialize, Serialize};
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::{
    net::{TcpListener, UdpSocket},
    sync::{Mutex, watch},
    task::JoinHandle,
    time::sleep,
};
use tower_http::trace::TraceLayer;
use tracing::{info, warn};

pub const DEFAULT_BIND_ADDR: &str = "0.0.0.0:8080";
pub const DEFAULT_DISCOVERY_UDP_BIND_ADDR: &str = "0.0.0.0:3478";
pub const DEFAULT_PRESENCE_LEASE_SECS: u64 = 30;
pub const DEFAULT_CLEANUP_INTERVAL_SECS: u64 = 5;
pub const DEFAULT_RELAY_SESSION_TIMEOUT_SECS: u64 = 60;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HubRuntimeConfig {
    pub bind_addr: String,
    pub discovery_udp_bind_addr: String,
    pub presence_lease_secs: u64,
    pub cleanup_interval_secs: u64,
    pub relay_session_timeout_secs: u64,
}

impl Default for HubRuntimeConfig {
    fn default() -> Self {
        Self {
            bind_addr: DEFAULT_BIND_ADDR.to_string(),
            discovery_udp_bind_addr: DEFAULT_DISCOVERY_UDP_BIND_ADDR.to_string(),
            presence_lease_secs: DEFAULT_PRESENCE_LEASE_SECS,
            cleanup_interval_secs: DEFAULT_CLEANUP_INTERVAL_SECS,
            relay_session_timeout_secs: DEFAULT_RELAY_SESSION_TIMEOUT_SECS,
        }
    }
}

impl HubRuntimeConfig {
    pub fn app_state(&self) -> AppStateModel {
        AppStateModel::new(self.presence_lease_secs, self.relay_session_timeout_secs)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HubRuntimeInfoModel {
    pub bind_addr: String,
    pub discovery_udp_bind_addr: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HubRuntimeStatusModel {
    pub bind_addr: String,
    pub discovery_udp_bind_addr: String,
    pub presence_lease_secs: u64,
    pub cleanup_interval_secs: u64,
    pub relay_session_timeout_secs: u64,
    pub peer_count: usize,
    pub relay_session_count: usize,
}

#[derive(Clone)]
pub struct HubRuntimeHandle {
    shared: Arc<HubRuntimeShared>,
}

struct HubRuntimeShared {
    config: HubRuntimeConfig,
    app_state: AppStateModel,
    shutdown_tx: watch::Sender<bool>,
    tasks: Mutex<Vec<JoinHandle<()>>>,
}

impl HubRuntimeHandle {
    pub async fn start(config: HubRuntimeConfig) -> Result<Self, String> {
        let app_state = config.app_state();
        Self::start_with_state(config, app_state).await
    }

    pub async fn start_with_state(
        config: HubRuntimeConfig,
        app_state: AppStateModel,
    ) -> Result<Self, String> {
        let addr: SocketAddr = config
            .bind_addr
            .parse()
            .map_err(|error| format!("invalid hub bind address '{}': {error}", config.bind_addr))?;
        let listener = TcpListener::bind(addr)
            .await
            .map_err(|error| format!("failed to bind hub listener on {addr}: {error}"))?;
        let discovery_udp_addr: SocketAddr =
            config.discovery_udp_bind_addr.parse().map_err(|error| {
                format!(
                    "invalid discovery UDP bind address '{}': {error}",
                    config.discovery_udp_bind_addr
                )
            })?;
        let discovery_socket = UdpSocket::bind(discovery_udp_addr)
            .await
            .map_err(|error| {
                format!(
                    "failed to bind discovery UDP socket on {discovery_udp_addr}: {error}"
                )
            })?;
        let secondary_discovery_udp_addr = SocketAddr::new(
            discovery_udp_addr.ip(),
            discovery_udp_addr.port() + 1,
        );
        let secondary_discovery_socket = UdpSocket::bind(secondary_discovery_udp_addr)
            .await
            .map_err(|error| {
                format!(
                    "failed to bind secondary discovery UDP socket on {secondary_discovery_udp_addr}: {error}"
                )
            })?;
        let (shutdown_tx, _) = watch::channel(false);
        let shared = Arc::new(HubRuntimeShared {
            config: config.clone(),
            app_state: app_state.clone(),
            shutdown_tx,
            tasks: Mutex::new(Vec::new()),
        });
        let handle = Self { shared };

        handle.spawn_cleanup_task().await;
        handle.spawn_discovery_udp_task(discovery_socket).await;
        handle
            .spawn_discovery_udp_task(secondary_discovery_socket)
            .await;
        handle.spawn_http_server(listener).await;

        info!("openhanse-core hub listening on {addr}");
        info!("openhanse-core UDP discovery listening on {discovery_udp_addr}");
        info!(
            "openhanse-core secondary UDP discovery listening on {secondary_discovery_udp_addr}"
        );
        Ok(handle)
    }

    pub async fn stop(&self) -> Result<(), String> {
        let _ = self.shared.shutdown_tx.send(true);
        let mut tasks = self.shared.tasks.lock().await;
        for task in tasks.drain(..) {
            let _ = task.await;
        }
        Ok(())
    }

    pub fn info(&self) -> HubRuntimeInfoModel {
        HubRuntimeInfoModel {
            bind_addr: self.shared.config.bind_addr.clone(),
            discovery_udp_bind_addr: self.shared.config.discovery_udp_bind_addr.clone(),
        }
    }

    pub async fn status(&self) -> HubRuntimeStatusModel {
        let guard = self.shared.app_state.inner.read().await;
        HubRuntimeStatusModel {
            bind_addr: self.shared.config.bind_addr.clone(),
            discovery_udp_bind_addr: self.shared.config.discovery_udp_bind_addr.clone(),
            presence_lease_secs: self.shared.config.presence_lease_secs,
            cleanup_interval_secs: self.shared.config.cleanup_interval_secs,
            relay_session_timeout_secs: self.shared.config.relay_session_timeout_secs,
            peer_count: guard.peers.len(),
            relay_session_count: guard.relay_sessions.len(),
        }
    }

    async fn spawn_cleanup_task(&self) {
        let state = self.shared.app_state.clone();
        let interval = Duration::from_secs(self.shared.config.cleanup_interval_secs);
        let mut shutdown_rx = self.shared.shutdown_tx.subscribe();
        let task = tokio::spawn(async move {
            cleanup_task(state, interval, &mut shutdown_rx).await;
        });
        self.shared.tasks.lock().await.push(task);
    }

    async fn spawn_discovery_udp_task(&self, socket: UdpSocket) {
        let mut shutdown_rx = self.shared.shutdown_tx.subscribe();
        let task = tokio::spawn(async move {
            discovery_udp_task_with_shutdown(socket, &mut shutdown_rx).await;
        });
        self.shared.tasks.lock().await.push(task);
    }

    async fn spawn_http_server(&self, listener: TcpListener) {
        let app = app_router(self.shared.app_state.clone());
        let mut shutdown_rx = self.shared.shutdown_tx.subscribe();
        let task = tokio::spawn(async move {
            let server = axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
                .with_graceful_shutdown(async move {
                    let _ = shutdown_rx.changed().await;
                });
            let _ = server.await;
        });
        self.shared.tasks.lock().await.push(task);
    }
}

pub async fn run(config: HubRuntimeConfig) -> Result<(), String> {
    let _handle = HubRuntimeHandle::start(config).await?;
    std::future::pending::<()>().await;
    Ok(())
}

pub async fn run_with_state(
    config: HubRuntimeConfig,
    app_state: AppStateModel,
) -> Result<(), String> {
    let _handle = HubRuntimeHandle::start_with_state(config, app_state).await?;
    std::future::pending::<()>().await;
    Ok(())
}

pub fn app_router(state: AppStateModel) -> Router {
    Router::new()
        .route("/health", get(health_endpoint))
        .route("/v1/peers/register", post(register_peer_endpoint))
        .route("/v1/peers/heartbeat", post(peer_heartbeat_endpoint))
        .route("/v1/peers/{peer_id}", get(get_peer_endpoint))
        .route("/v1/connect", post(connect_peer_endpoint))
        .route("/v1/relay/attach", post(relay_attach_endpoint))
        .route("/v1/relay/send", post(relay_send_endpoint))
        .route("/v1/relay/poll", post(relay_poll_endpoint))
        .with_state(state)
        .layer(TraceLayer::new_for_http())
}

async fn cleanup_task(
    state: AppStateModel,
    interval: Duration,
    shutdown_rx: &mut watch::Receiver<bool>,
) {
    loop {
        tokio::select! {
            _ = sleep(interval) => {
                let mut guard = state.inner.write().await;
                let peers_before = guard.peers.len();
                let sessions_before = guard.relay_sessions.len();
                guard.prune_expired();

                if guard.peers.len() != peers_before || guard.relay_sessions.len() != sessions_before {
                    warn!(
                        peers = guard.peers.len(),
                        relay_sessions = guard.relay_sessions.len(),
                        "pruned expired state"
                    );
                }
            }
            _ = shutdown_rx.changed() => break,
        }
    }
}

pub async fn discovery_udp_task(socket: UdpSocket) {
    let (_shutdown_tx, mut shutdown_rx) = watch::channel(false);
    discovery_udp_task_with_shutdown(socket, &mut shutdown_rx).await;
}

async fn discovery_udp_task_with_shutdown(socket: UdpSocket, shutdown_rx: &mut watch::Receiver<bool>) {
    let mut buffer = [0_u8; 1024];
    loop {
        tokio::select! {
            received = socket.recv_from(&mut buffer) => {
                let Ok((len, remote_addr)) = received else {
                    continue;
                };
                let Ok(request) =
                    serde_json::from_slice::<DiscoveryProbeRequestModel>(&buffer[..len])
                else {
                    continue;
                };
                let response = DiscoveryProbeResponseModel {
                    transaction_id: request.transaction_id,
                    observed_addr: remote_addr.to_string(),
                    transport_protocol: TransportProtocolEnum::DirectUdp,
                };
                let Ok(payload) = serde_json::to_vec(&response) else {
                    continue;
                };
                let _ = socket.send_to(&payload, remote_addr).await;
            }
            _ = shutdown_rx.changed() => break,
        }
    }
}
