use crate::hub::model::{
    api_error_model::ApiErrorModel,
    app_state_model::AppStateModel,
};
use axum::{Json, extract::State};
use crate::model::connect_model::{ConnectDecisionModel, ConnectRequestModel};
use tracing::info;

pub async fn connect_peer_endpoint(
    State(state): State<AppStateModel>,
    Json(request): Json<ConnectRequestModel>,
) -> Result<Json<ConnectDecisionModel>, ApiErrorModel> {
    let source_peer_id = request.source_peer_id.clone();
    let target_peer_id = request.target_peer_id.clone();
    let mut guard = state.inner.write().await;
    let decision = guard.connect(request)?;
    match &decision {
        ConnectDecisionModel::Direct { .. } => info!(
            source_peer_id,
            target_peer_id,
            delivery_mode = "direct",
            "connect decision issued"
        ),
        ConnectDecisionModel::Relay { relay } => info!(
            source_peer_id,
            target_peer_id,
            delivery_mode = "relay",
            relay_session_id = %relay.relay_session_id,
            "connect decision issued"
        ),
    }
    Ok(Json(decision))
}
