use crate::hub::model::{
    api_error_model::ApiErrorModel,
    app_state_model::AppStateModel,
};
use axum::{
    Json,
    extract::{ConnectInfo, Path, State},
};
use crate::model::peer_model::{
    HeartbeatRequestModel, PeerLookupResponseModel, RegisterPeerRequestModel,
    RegisterPeerResponseModel,
};
use tracing::info;

pub async fn register_peer_endpoint(
    ConnectInfo(remote_addr): ConnectInfo<std::net::SocketAddr>,
    State(state): State<AppStateModel>,
    Json(request): Json<RegisterPeerRequestModel>,
) -> Result<Json<RegisterPeerResponseModel>, ApiErrorModel> {
    if request.peer_id.trim().is_empty() {
        return Err(ApiErrorModel::bad_request("peer_id must not be empty"));
    }
    if request.device_key.trim().is_empty() {
        return Err(ApiErrorModel::bad_request("device_key must not be empty"));
    }

    let mut guard = state.inner.write().await;
    let peer = guard.register_peer(request, Some(remote_addr));
    info!("registered peer {}", peer.peer_id);

    Ok(Json(RegisterPeerResponseModel {
        lease_seconds: guard.presence_lease.as_secs(),
        peer,
    }))
}

pub async fn peer_heartbeat_endpoint(
    State(state): State<AppStateModel>,
    Json(request): Json<HeartbeatRequestModel>,
) -> Result<Json<RegisterPeerResponseModel>, ApiErrorModel> {
    let mut guard = state.inner.write().await;
    let peer = guard
        .heartbeat(&request.peer_id)
        .ok_or_else(|| ApiErrorModel::not_found(format!("peer '{}' is offline", request.peer_id)))?;

    Ok(Json(RegisterPeerResponseModel {
        lease_seconds: guard.presence_lease.as_secs(),
        peer,
    }))
}

pub async fn get_peer_endpoint(
    State(state): State<AppStateModel>,
    Path(peer_id): Path<String>,
) -> Result<Json<PeerLookupResponseModel>, ApiErrorModel> {
    let mut guard = state.inner.write().await;
    let peer = guard
        .lookup_peer(&peer_id)
        .ok_or_else(|| ApiErrorModel::not_found(format!("peer '{}' is offline", peer_id)))?;

    Ok(Json(PeerLookupResponseModel { peer }))
}
