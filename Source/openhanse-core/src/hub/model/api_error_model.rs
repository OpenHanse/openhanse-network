use axum::{Json, http::StatusCode, response::IntoResponse};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct ErrorResponseModel {
    pub error: String,
}

#[derive(Debug)]
pub struct ApiErrorModel {
    pub status: StatusCode,
    pub message: String,
}

impl ApiErrorModel {
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.into(),
        }
    }
}

impl IntoResponse for ApiErrorModel {
    fn into_response(self) -> axum::response::Response {
        (
            self.status,
            Json(ErrorResponseModel {
                error: self.message,
            }),
        )
            .into_response()
    }
}
