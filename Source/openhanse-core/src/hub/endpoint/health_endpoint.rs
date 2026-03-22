use crate::hub::util::time_util::TimeUtil;
use axum::Json;
use crate::model::health_model::HealthResponseModel;

pub async fn health_endpoint() -> Json<HealthResponseModel> {
    Json(HealthResponseModel {
        status: "ok",
        server_time_unix_ms: TimeUtil::unix_time_ms(),
    })
}
