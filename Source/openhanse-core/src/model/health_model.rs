use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HealthResponseModel {
    pub status: &'static str,
    pub server_time_unix_ms: u64,
}
