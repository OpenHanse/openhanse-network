use crate::{
    GatewayRuntimeConfig, GatewayRuntimeHandle, GatewayRuntimeInfoModel, GatewayRuntimeStatusModel,
    HubRuntimeConfig, HubRuntimeHandle, HubRuntimeInfoModel, HubRuntimeStatusModel,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PeerModeEnum {
    Both,
    Hub,
    Gateway,
}

impl PeerModeEnum {
    pub fn runs_gateway(self) -> bool {
        matches!(self, Self::Both | Self::Gateway)
    }

    pub fn runs_hub(self) -> bool {
        matches!(self, Self::Both | Self::Hub)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Both => "both",
            Self::Hub => "hub",
            Self::Gateway => "gateway",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PeerRuntimeConfig {
    pub peer_mode: PeerModeEnum,
    pub gateway: GatewayRuntimeConfig,
    pub hub: HubRuntimeConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PeerRuntimeInfoModel {
    pub peer_mode: String,
    pub gateway: Option<GatewayRuntimeInfoModel>,
    pub hub: Option<HubRuntimeInfoModel>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PeerRuntimeStatusModel {
    pub peer_mode: String,
    pub gateway: Option<GatewayRuntimeStatusModel>,
    pub hub: Option<HubRuntimeStatusModel>,
}

#[derive(Clone)]
pub struct PeerRuntimeHandle {
    peer_mode: PeerModeEnum,
    gateway_runtime: Option<GatewayRuntimeHandle>,
    hub_runtime: Option<HubRuntimeHandle>,
}

impl PeerRuntimeHandle {
    pub async fn start(config: PeerRuntimeConfig) -> Result<Self, String> {
        let hub_runtime = if config.peer_mode.runs_hub() {
            Some(HubRuntimeHandle::start(config.hub).await?)
        } else {
            None
        };
        let gateway_runtime = if config.peer_mode.runs_gateway() {
            Some(GatewayRuntimeHandle::start(config.gateway).await?)
        } else {
            None
        };
        Ok(Self {
            peer_mode: config.peer_mode,
            gateway_runtime,
            hub_runtime,
        })
    }

    pub async fn stop(&self) -> Result<(), String> {
        if let Some(gateway_runtime) = &self.gateway_runtime {
            gateway_runtime.stop().await?;
        }
        if let Some(hub_runtime) = &self.hub_runtime {
            hub_runtime.stop().await?;
        }
        Ok(())
    }

    pub fn info(&self) -> PeerRuntimeInfoModel {
        PeerRuntimeInfoModel {
            peer_mode: self.peer_mode.as_str().to_string(),
            gateway: self.gateway_runtime.as_ref().map(|runtime| runtime.info()),
            hub: self.hub_runtime.as_ref().map(|runtime| runtime.info()),
        }
    }

    pub async fn status(&self) -> PeerRuntimeStatusModel {
        PeerRuntimeStatusModel {
            peer_mode: self.peer_mode.as_str().to_string(),
            gateway: match &self.gateway_runtime {
                Some(runtime) => Some(runtime.status().await),
                None => None,
            },
            hub: match &self.hub_runtime {
                Some(runtime) => Some(runtime.status().await),
                None => None,
            },
        }
    }

    pub fn gateway_runtime(&self) -> Option<GatewayRuntimeHandle> {
        self.gateway_runtime.clone()
    }

    pub fn hub_runtime(&self) -> Option<HubRuntimeHandle> {
        self.hub_runtime.clone()
    }

    pub fn peer_mode(&self) -> PeerModeEnum {
        self.peer_mode
    }
}
