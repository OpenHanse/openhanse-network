use crate::hub::model::{
    api_error_model::ApiErrorModel,
    app_state_model::AppStateModel,
};
use axum::{Json, extract::State};
use crate::model::relay_model::{
    RelayAttachRequestModel, RelayAttachResponseModel, RelayPollRequestModel,
    RelayPollResponseModel, RelaySendRequestModel, RelaySendResponseModel,
};
use tracing::info;

pub async fn relay_attach_endpoint(
    State(state): State<AppStateModel>,
    Json(request): Json<RelayAttachRequestModel>,
) -> Result<Json<RelayAttachResponseModel>, ApiErrorModel> {
    let mut guard = state.inner.write().await;
    let response = guard.attach_relay_peer(request.relay_session_id, &request.peer_id)?;
    info!(
        relay_session_id = %response.relay_session_id,
        peer_id = response.peer_id,
        counterpart_peer_id = response.counterpart_peer_id,
        "relay peer attached"
    );
    Ok(Json(response))
}

pub async fn relay_send_endpoint(
    State(state): State<AppStateModel>,
    Json(request): Json<RelaySendRequestModel>,
) -> Result<Json<RelaySendResponseModel>, ApiErrorModel> {
    let mut guard = state.inner.write().await;
    let response = guard.relay_send(request)?;
    info!(
        relay_session_id = %response.relay_session_id,
        recipient_peer_id = response.recipient_peer_id,
        queued_messages = response.queued_messages,
        "relay message queued"
    );
    Ok(Json(response))
}

pub async fn relay_poll_endpoint(
    State(state): State<AppStateModel>,
    Json(request): Json<RelayPollRequestModel>,
) -> Result<Json<RelayPollResponseModel>, ApiErrorModel> {
    let mut guard = state.inner.write().await;
    let messages = guard.poll_relay_messages(&request.peer_id)?;
    if !messages.is_empty() {
        info!(
            peer_id = request.peer_id,
            message_count = messages.len(),
            "relay messages delivered"
        );
    }
    Ok(Json(RelayPollResponseModel { messages }))
}
