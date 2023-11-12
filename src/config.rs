use serde::Deserialize;
use zenoh::config::Config as ZenohConfig;

use crate::FoxgloveBridgeError;

#[derive(Debug, Deserialize)]
pub struct Configuration {
    pub protobuf_subscriptions: Vec<ProtobufSubscriptioin>,
    pub json_subscriptions: Vec<JsonSubscription>,
    pub zenoh: FoxgloveBridgeZenohConfig,
}

#[derive(Debug, Deserialize)]
pub struct ProtobufSubscriptioin {
    pub topic: String,
    pub proto_type: String,
}

#[derive(Debug, Deserialize)]
pub struct JsonSubscription {
    pub topic: String,
    pub type_name: String,
    pub json_schema_name: Option<String>,
    pub latched: Option<bool>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct FoxgloveBridgeZenohConfig {
    pub connect: Vec<zenoh_config::EndPoint>,
    pub listen: Vec<zenoh_config::EndPoint>,
    pub config_path: Option<String>,
}

impl FoxgloveBridgeZenohConfig {
    pub fn get_zenoh_config(&self) -> anyhow::Result<ZenohConfig> {
        let mut config = if let Some(conf_file) = &self.config_path {
            ZenohConfig::from_file(conf_file).map_err(FoxgloveBridgeError::ZenohError)?
        } else {
            ZenohConfig::default()
        };
        if !self.connect.is_empty() {
            config.connect.endpoints = self.connect.clone();
        }
        if !self.listen.is_empty() {
            config.listen.endpoints = self.listen.clone();
        }
        Ok(config)
    }
}
