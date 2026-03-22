use crate::hub::model::server_state_model::ServerStateModel;
use std::{sync::Arc, time::Duration};
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct AppStateModel {
    pub inner: Arc<RwLock<ServerStateModel>>,
}

impl AppStateModel {
    pub fn new(presence_lease_secs: u64, relay_session_timeout_secs: u64) -> Self {
        Self {
            inner: Arc::new(RwLock::new(ServerStateModel::new(
                Duration::from_secs(presence_lease_secs),
                Duration::from_secs(relay_session_timeout_secs),
            ))),
        }
    }
}
