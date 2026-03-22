use axum::{
    Json, Router,
    extract::{Query, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use openhanse_core::{
    CommunicationModeEnum, ErrorResponseModel, GatewayRuntimeConfig, HubRuntimeConfig,
    InboxEntryModel, PeerModeEnum, PeerRuntimeConfig, PeerRuntimeHandle, SendMessageRequestModel,
    SendMessageResponseModel, UiEventModel,
};
use openhanse_core::model::{
    connect_model::ConnectDecisionModel, peer_model::PeerLookupResponseModel,
};
use serde::{Deserialize, Serialize};
use std::{
    ffi::{CStr, CString, c_char},
    path::PathBuf,
    sync::{Arc, Mutex as StdMutex, OnceLock},
};
use tokio::{
    net::TcpListener,
    sync::{Mutex, watch},
    task::JoinHandle,
};
use tower_http::cors::{Any, CorsLayer};

const INDEX_HTML: &str = include_str!("../assets/WebUI/index.html");
const APP_JS: &str = include_str!("../assets/WebUI/app.js");
const APP_CSS: &str = include_str!("../assets/WebUI/app.css");
const OH_LOG_JS: &str = include_str!("../assets/WebUI/components/oh-log.js");
const OH_PROMPT_JS: &str = include_str!("../assets/WebUI/components/oh-prompt.js");
const OH_SHELL_JS: &str = include_str!("../assets/WebUI/components/oh-shell.js");
const OH_STATUS_JS: &str = include_str!("../assets/WebUI/components/oh-status.js");

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GatewayWebRuntimeConfig {
    pub peer_id: String,
    pub device_key: String,
    pub display_name: Option<String>,
    pub target_peer_id: String,
    pub server_base_url: String,
    pub direct_bind_host: String,
    pub direct_bind_port: u16,
    #[serde(default = "default_peer_mode")]
    pub peer_mode: PeerModeEnum,
    #[serde(default = "default_communication_mode")]
    pub communication_mode: CommunicationModeEnum,
    pub ui_bind_port: u16,
    pub heartbeat_interval_secs: u64,
    pub storage_dir: PathBuf,
}

impl GatewayWebRuntimeConfig {
    pub fn peer_config(&self) -> Result<PeerRuntimeConfig, String> {
        Ok(PeerRuntimeConfig {
            peer_mode: self.peer_mode,
            gateway: GatewayRuntimeConfig {
                peer_id: self.peer_id.clone(),
                device_key: self.device_key.clone(),
                display_name: self.display_name.clone(),
                target_peer_id: self.target_peer_id.clone(),
                server_base_url: self.server_base_url.clone(),
                direct_bind_host: self.direct_bind_host.clone(),
                direct_bind_port: self.direct_bind_port,
                communication_mode: self.communication_mode,
                heartbeat_interval_secs: self.heartbeat_interval_secs,
                storage_dir: self.storage_dir.clone(),
            },
            hub: hub_runtime_config(&self.server_base_url)?,
        })
    }
}

fn default_peer_mode() -> PeerModeEnum {
    PeerModeEnum::Gateway
}

fn default_communication_mode() -> CommunicationModeEnum {
    CommunicationModeEnum::Auto
}

fn hub_runtime_config(server_base_url: &str) -> Result<HubRuntimeConfig, String> {
    let (host, port) = host_and_port_from_url(server_base_url)?;
    let bind_host = if host == "127.0.0.1" || host.eq_ignore_ascii_case("localhost") {
        host
    } else {
        "0.0.0.0".to_string()
    };
    Ok(HubRuntimeConfig {
        bind_addr: format!("{bind_host}:{port}"),
        discovery_udp_bind_addr: format!("{bind_host}:3478"),
        ..HubRuntimeConfig::default()
    })
}

fn host_and_port_from_url(url: &str) -> Result<(String, u16), String> {
    let without_scheme = url
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(url);
    let authority = without_scheme
        .split('/')
        .next()
        .ok_or_else(|| format!("invalid server URL: {url}"))?;
    let (host, port) = authority
        .rsplit_once(':')
        .ok_or_else(|| format!("server URL must include a port: {url}"))?;
    let port = port
        .parse::<u16>()
        .map_err(|_| format!("invalid server port in URL: {url}"))?;
    Ok((host.to_string(), port))
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GatewayWebRuntimeInfoModel {
    pub peer_id: String,
    pub target_peer_id: String,
    pub peer_mode: String,
    pub server_base_url: String,
    pub direct_base_url: String,
    pub message_endpoint: String,
    pub ui_base_url: String,
    pub hub_bind_addr: Option<String>,
    pub discovery_udp_bind_addr: Option<String>,
    pub storage_dir: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GatewayWebRuntimeStatusModel {
    pub peer_id: String,
    pub display_name: Option<String>,
    pub target_peer_id: String,
    pub peer_mode: String,
    pub communication_mode: String,
    pub server_base_url: String,
    pub direct_base_url: String,
    pub message_endpoint: String,
    pub ui_base_url: String,
    pub hub_bind_addr: Option<String>,
    pub discovery_udp_bind_addr: Option<String>,
    pub hub_peer_count: usize,
    pub hub_relay_session_count: usize,
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

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct EventQueryModel {
    pub since_event_id: Option<u64>,
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
pub struct PollResponseModel {
    pub status: GatewayWebRuntimeStatusModel,
    pub inbox: Vec<InboxEntryModel>,
    pub events: Vec<UiEventModel>,
}

#[derive(Clone)]
pub struct GatewayWebRuntimeHandle {
    shared: Arc<GatewayWebRuntimeShared>,
}

struct GatewayWebRuntimeShared {
    peer_runtime: PeerRuntimeHandle,
    ui_base_url: String,
    shutdown_tx: watch::Sender<bool>,
    tasks: Mutex<Vec<JoinHandle<()>>>,
}

#[derive(Clone)]
struct UiApiAppState {
    runtime: GatewayWebRuntimeHandle,
}

impl GatewayWebRuntimeHandle {
    pub async fn start(config: GatewayWebRuntimeConfig) -> Result<Self, String> {
        let ui_listener = TcpListener::bind(("127.0.0.1", config.ui_bind_port))
            .await
            .map_err(|error| {
                format!(
                    "failed to bind UI API on 127.0.0.1:{}: {error}",
                    config.ui_bind_port
                )
            })?;
        let ui_address = ui_listener
            .local_addr()
            .map_err(|error| format!("failed to inspect UI listener: {error}"))?;

        let peer_runtime = PeerRuntimeHandle::start(config.peer_config()?).await?;
        let (shutdown_tx, _) = watch::channel(false);
        let handle = Self {
            shared: Arc::new(GatewayWebRuntimeShared {
                peer_runtime,
                ui_base_url: format!("http://127.0.0.1:{}", ui_address.port()),
                shutdown_tx,
                tasks: Mutex::new(Vec::new()),
            }),
        };

        handle.spawn_ui_api(ui_listener).await;
        Ok(handle)
    }

    pub async fn stop(&self) -> Result<(), String> {
        let _ = self.shared.shutdown_tx.send(true);
        let mut tasks = self.shared.tasks.lock().await;
        for task in tasks.drain(..) {
            let _ = task.await;
        }
        drop(tasks);
        self.shared.peer_runtime.stop().await
    }

    pub fn info(&self) -> GatewayWebRuntimeInfoModel {
        let core_info = self.shared.peer_runtime.info();
        let gateway_info = core_info.gateway;
        let hub_info = core_info.hub;
        GatewayWebRuntimeInfoModel {
            peer_id: gateway_info
                .as_ref()
                .map(|info| info.peer_id.clone())
                .unwrap_or_default(),
            target_peer_id: gateway_info
                .as_ref()
                .map(|info| info.target_peer_id.clone())
                .unwrap_or_default(),
            peer_mode: core_info.peer_mode,
            server_base_url: gateway_info
                .as_ref()
                .map(|info| info.server_base_url.clone())
                .unwrap_or_default(),
            direct_base_url: gateway_info
                .as_ref()
                .map(|info| info.direct_base_url.clone())
                .unwrap_or_default(),
            message_endpoint: gateway_info
                .as_ref()
                .map(|info| info.message_endpoint.clone())
                .unwrap_or_default(),
            ui_base_url: self.shared.ui_base_url.clone(),
            hub_bind_addr: hub_info.as_ref().map(|info| info.bind_addr.clone()),
            discovery_udp_bind_addr: hub_info
                .as_ref()
                .map(|info| info.discovery_udp_bind_addr.clone()),
            storage_dir: gateway_info
                .as_ref()
                .map(|info| info.storage_dir.clone())
                .unwrap_or_default(),
        }
    }

    pub async fn status(&self) -> GatewayWebRuntimeStatusModel {
        let core_status = self.shared.peer_runtime.status().await;
        let gateway_status = core_status.gateway;
        let hub_status = core_status.hub;
        GatewayWebRuntimeStatusModel {
            peer_id: gateway_status
                .as_ref()
                .map(|status| status.peer_id.clone())
                .unwrap_or_default(),
            display_name: gateway_status.as_ref().and_then(|status| status.display_name.clone()),
            target_peer_id: gateway_status
                .as_ref()
                .map(|status| status.target_peer_id.clone())
                .unwrap_or_default(),
            peer_mode: core_status.peer_mode,
            communication_mode: gateway_status
                .as_ref()
                .map(|status| status.communication_mode.clone())
                .unwrap_or_else(|| "disabled".to_string()),
            server_base_url: gateway_status
                .as_ref()
                .map(|status| status.server_base_url.clone())
                .unwrap_or_default(),
            direct_base_url: gateway_status
                .as_ref()
                .map(|status| status.direct_base_url.clone())
                .unwrap_or_default(),
            message_endpoint: gateway_status
                .as_ref()
                .map(|status| status.message_endpoint.clone())
                .unwrap_or_default(),
            ui_base_url: self.shared.ui_base_url.clone(),
            hub_bind_addr: hub_status.as_ref().map(|status| status.bind_addr.clone()),
            discovery_udp_bind_addr: hub_status
                .as_ref()
                .map(|status| status.discovery_udp_bind_addr.clone()),
            hub_peer_count: hub_status.as_ref().map(|status| status.peer_count).unwrap_or(0),
            hub_relay_session_count: hub_status
                .as_ref()
                .map(|status| status.relay_session_count)
                .unwrap_or(0),
            heartbeat_interval_secs: gateway_status
                .as_ref()
                .map(|status| status.heartbeat_interval_secs)
                .unwrap_or(0),
            heartbeat_state: gateway_status
                .as_ref()
                .map(|status| status.heartbeat_state.clone())
                .unwrap_or_else(|| "disabled".to_string()),
            last_registered_at_unix_ms: gateway_status
                .as_ref()
                .and_then(|status| status.last_registered_at_unix_ms),
            last_heartbeat_at_unix_ms: gateway_status
                .as_ref()
                .and_then(|status| status.last_heartbeat_at_unix_ms),
            last_error: gateway_status.as_ref().and_then(|status| status.last_error.clone()),
            last_delivery_mode: gateway_status
                .as_ref()
                .and_then(|status| status.last_delivery_mode.clone()),
            last_delivery_summary: gateway_status
                .as_ref()
                .and_then(|status| status.last_delivery_summary.clone()),
            inbox_count: gateway_status
                .as_ref()
                .map(|status| status.inbox_count)
                .unwrap_or(0),
            event_count: gateway_status
                .as_ref()
                .map(|status| status.event_count)
                .unwrap_or(0),
            direct_sent_count: gateway_status
                .as_ref()
                .map(|status| status.direct_sent_count)
                .unwrap_or(0),
            relay_sent_count: gateway_status
                .as_ref()
                .map(|status| status.relay_sent_count)
                .unwrap_or(0),
            direct_received_count: gateway_status
                .as_ref()
                .map(|status| status.direct_received_count)
                .unwrap_or(0),
            relay_received_count: gateway_status
                .as_ref()
                .map(|status| status.relay_received_count)
                .unwrap_or(0),
        }
    }

    pub async fn list_inbox(&self) -> Vec<InboxEntryModel> {
        match self.shared.peer_runtime.gateway_runtime() {
            Some(runtime) => runtime.list_inbox().await,
            None => Vec::new(),
        }
    }

    pub async fn events_since(&self, since_event_id: Option<u64>) -> Vec<UiEventModel> {
        match self.shared.peer_runtime.gateway_runtime() {
            Some(runtime) => runtime.events_since(since_event_id).await,
            None => Vec::new(),
        }
    }

    pub async fn poll(&self, since_event_id: Option<u64>) -> PollResponseModel {
        PollResponseModel {
            status: self.status().await,
            inbox: self.list_inbox().await,
            events: self.events_since(since_event_id).await,
        }
    }

    pub async fn lookup_target(&self) -> Result<PeerLookupResponseModel, String> {
        self.shared
            .peer_runtime
            .gateway_runtime()
            .ok_or_else(|| "peer mode does not enable gateway operations".to_string())?
            .lookup_target()
            .await
    }

    pub async fn connect_target(&self) -> Result<ConnectDecisionModel, String> {
        self.shared
            .peer_runtime
            .gateway_runtime()
            .ok_or_else(|| "peer mode does not enable gateway operations".to_string())?
            .connect_target()
            .await
    }

    pub async fn send_message(
        &self,
        message: impl Into<String>,
    ) -> Result<SendMessageResponseModel, String> {
        self.shared
            .peer_runtime
            .gateway_runtime()
            .ok_or_else(|| "peer mode does not enable gateway operations".to_string())?
            .send_message(message)
            .await
    }

    async fn spawn_ui_api(&self, listener: TcpListener) {
        let cors = CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(Any)
            .allow_headers(Any);
        let app = Router::new()
            .route("/", get(index_page))
            .route("/index.html", get(index_page))
            .route("/app.js", get(app_js))
            .route("/app.css", get(app_css))
            .route("/components/oh-log.js", get(oh_log_js))
            .route("/components/oh-prompt.js", get(oh_prompt_js))
            .route("/components/oh-shell.js", get(oh_shell_js))
            .route("/components/oh-status.js", get(oh_status_js))
            .route("/api/status", get(status_endpoint))
            .route("/api/inbox", get(inbox_endpoint))
            .route("/api/events", get(events_endpoint))
            .route("/api/poll", get(poll_endpoint))
            .route("/api/lookup", post(lookup_endpoint))
            .route("/api/connect", post(connect_endpoint))
            .route("/api/messages", post(send_message_endpoint))
            .layer(cors)
            .with_state(UiApiAppState {
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
}

async fn status_endpoint(
    State(state): State<UiApiAppState>,
) -> Json<GatewayWebRuntimeStatusModel> {
    Json(state.runtime.status().await)
}

async fn inbox_endpoint(State(state): State<UiApiAppState>) -> Json<InboxListResponseModel> {
    Json(InboxListResponseModel {
        inbox: state.runtime.list_inbox().await,
    })
}

async fn events_endpoint(
    State(state): State<UiApiAppState>,
    Query(query): Query<EventQueryModel>,
) -> Json<EventsResponseModel> {
    Json(EventsResponseModel {
        events: state.runtime.events_since(query.since_event_id).await,
    })
}

async fn poll_endpoint(
    State(state): State<UiApiAppState>,
    Query(query): Query<EventQueryModel>,
) -> Json<PollResponseModel> {
    Json(state.runtime.poll(query.since_event_id).await)
}

async fn lookup_endpoint(
    State(state): State<UiApiAppState>,
) -> Result<Json<PeerLookupResponseModel>, (StatusCode, Json<ErrorResponseModel>)> {
    state.runtime.lookup_target().await.map(Json).map_err(error_response)
}

async fn connect_endpoint(
    State(state): State<UiApiAppState>,
) -> Result<Json<ConnectDecisionModel>, (StatusCode, Json<ErrorResponseModel>)> {
    state
        .runtime
        .connect_target()
        .await
        .map(Json)
        .map_err(error_response)
}

async fn send_message_endpoint(
    State(state): State<UiApiAppState>,
    Json(request): Json<SendMessageRequestModel>,
) -> Result<Json<SendMessageResponseModel>, (StatusCode, Json<ErrorResponseModel>)> {
    state
        .runtime
        .send_message(request.message)
        .await
        .map(Json)
        .map_err(error_response)
}

fn text_response(content_type: &'static str, body: &'static str) -> Response {
    (
        [(header::CONTENT_TYPE, HeaderValue::from_static(content_type))],
        body,
    )
        .into_response()
}

async fn index_page() -> Response {
    text_response("text/html; charset=utf-8", INDEX_HTML)
}

async fn app_js() -> Response {
    text_response("text/javascript; charset=utf-8", APP_JS)
}

async fn app_css() -> Response {
    text_response("text/css; charset=utf-8", APP_CSS)
}

async fn oh_log_js() -> Response {
    text_response("text/javascript; charset=utf-8", OH_LOG_JS)
}

async fn oh_prompt_js() -> Response {
    text_response("text/javascript; charset=utf-8", OH_PROMPT_JS)
}

async fn oh_shell_js() -> Response {
    text_response("text/javascript; charset=utf-8", OH_SHELL_JS)
}

async fn oh_status_js() -> Response {
    text_response("text/javascript; charset=utf-8", OH_STATUS_JS)
}

fn error_response(message: String) -> (StatusCode, Json<ErrorResponseModel>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponseModel { error: message }),
    )
}

struct HostRuntime {
    tokio_runtime: tokio::runtime::Runtime,
    peer_runtime: GatewayWebRuntimeHandle,
}

#[derive(Serialize)]
struct BridgeResponse<T: Serialize> {
    ok: bool,
    data: Option<T>,
    error: Option<String>,
}

static HOST_RUNTIME: OnceLock<StdMutex<Option<HostRuntime>>> = OnceLock::new();

fn runtime_slot() -> &'static StdMutex<Option<HostRuntime>> {
    HOST_RUNTIME.get_or_init(|| StdMutex::new(None))
}

#[unsafe(no_mangle)]
pub extern "C" fn openhanse_start(config_json: *const c_char) -> *mut c_char {
    let response = match parse_config(config_json) {
        Ok(config) => match tokio::runtime::Runtime::new() {
            Ok(tokio_runtime) => {
                let peer_runtime =
                    match tokio_runtime.block_on(GatewayWebRuntimeHandle::start(config)) {
                        Ok(peer_runtime) => peer_runtime,
                        Err(error) => {
                            return response_to_c_string(BridgeResponse::<GatewayWebRuntimeInfoModel> {
                                ok: false,
                                data: None,
                                error: Some(error),
                            });
                        }
                    };
                let info = peer_runtime.info();
                match runtime_slot().lock() {
                    Ok(mut slot) => {
                        if let Some(existing) = slot.take() {
                            let _ = existing
                                .tokio_runtime
                                .block_on(existing.peer_runtime.stop());
                        }
                        *slot = Some(HostRuntime {
                            tokio_runtime,
                            peer_runtime,
                        });
                        response_to_c_string(BridgeResponse {
                            ok: true,
                            data: Some(info),
                            error: None,
                        })
                    }
                    Err(error) => response_to_c_string(BridgeResponse::<GatewayWebRuntimeInfoModel> {
                        ok: false,
                        data: None,
                        error: Some(format!("failed to lock runtime: {error}")),
                    }),
                }
            }
            Err(error) => response_to_c_string(BridgeResponse::<GatewayWebRuntimeInfoModel> {
                ok: false,
                data: None,
                error: Some(format!("failed to create tokio runtime: {error}")),
            }),
        },
        Err(error) => response_to_c_string(BridgeResponse::<GatewayWebRuntimeInfoModel> {
            ok: false,
            data: None,
            error: Some(error),
        }),
    };

    response
}

#[unsafe(no_mangle)]
pub extern "C" fn openhanse_runtime_status() -> *mut c_char {
    match runtime_slot().lock() {
        Ok(slot) => {
            let Some(runtime) = slot.as_ref() else {
                return response_to_c_string(BridgeResponse::<serde_json::Value> {
                    ok: false,
                    data: None,
                    error: Some("runtime is not running".to_string()),
                });
            };
            let status = runtime.tokio_runtime.block_on(runtime.peer_runtime.status());
            response_to_c_string(BridgeResponse {
                ok: true,
                data: Some(status),
                error: None,
            })
        }
        Err(error) => response_to_c_string(BridgeResponse::<serde_json::Value> {
            ok: false,
            data: None,
            error: Some(format!("failed to lock runtime: {error}")),
        }),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn openhanse_stop() {
    if let Ok(mut slot) = runtime_slot().lock()
        && let Some(runtime) = slot.take()
    {
        let _ = runtime
            .tokio_runtime
            .block_on(runtime.peer_runtime.stop());
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn openhanse_string_free(value: *mut c_char) {
    if !value.is_null() {
        unsafe {
            drop(CString::from_raw(value));
        }
    }
}

fn parse_config(config_json: *const c_char) -> Result<GatewayWebRuntimeConfig, String> {
    if config_json.is_null() {
        return Err("config_json must not be null".to_string());
    }

    let raw = unsafe { CStr::from_ptr(config_json) }
        .to_str()
        .map_err(|error| format!("invalid config string: {error}"))?;
    serde_json::from_str(raw).map_err(|error| format!("invalid config JSON: {error}"))
}

fn response_to_c_string<T: Serialize>(response: BridgeResponse<T>) -> *mut c_char {
    let json = serde_json::to_string(&response).unwrap_or_else(|error| {
        format!(
            "{{\"ok\":false,\"data\":null,\"error\":\"failed to serialize response: {error}\"}}"
        )
    });
    CString::new(json)
        .expect("bridge response must not contain interior nul bytes")
        .into_raw()
}
