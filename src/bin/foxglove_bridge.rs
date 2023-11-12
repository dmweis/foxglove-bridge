use anyhow::Context;
use clap::Parser;
use foxglove_ws::{Channel, FoxgloveWebSocket};
use mcap::records::system_time_to_nanos;
use once_cell::sync::Lazy;
use prost_reflect::{DescriptorPool, MessageDescriptor};
use std::{
    collections::HashMap, net::SocketAddr, path::PathBuf, sync::Arc, sync::OnceLock,
    time::SystemTime,
};
use tokio::signal;
use tracing::info;
use zenoh::prelude::r#async::*;

use foxglove_bridge::{setup_tracing, FoxgloveBridgeError};

static FILE_DESCRIPTOR_SET: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/file_descriptor_set.bin"));

static DESCRIPTOR_POOL: Lazy<DescriptorPool> = Lazy::new(|| {
    DescriptorPool::decode(FILE_DESCRIPTOR_SET).expect("Failed to load file descriptor set")
});

/// protobuf
pub mod foxglove {
    #![allow(non_snake_case)]
    include!(concat!(env!("OUT_DIR"), "/foxglove.rs"));
}

pub mod hopper {
    #![allow(non_snake_case)]
    include!(concat!(env!("OUT_DIR"), "/hopper.rs"));
}

fn json_schema_table() -> &'static HashMap<String, String> {
    static INSTANCE: OnceLock<HashMap<String, String>> = OnceLock::new();
    INSTANCE.get_or_init(|| {
        let mut m = HashMap::new();
        m.insert("GENERIC_JSON".to_owned(), GENERIC_JSON_SCHEMA.to_owned());
        m.insert(
            "IKEA_DIMMER_JSON_SCHEMA".to_owned(),
            IKEA_DIMMER_JSON_SCHEMA.to_owned(),
        );
        m.insert(
            "MOTION_SENSOR_JSON_SCHEMA".to_owned(),
            MOTION_SENSOR_JSON_SCHEMA.to_owned(),
        );
        m.insert(
            "CONTACT_SENSOR_JSON_SCHEMA".to_owned(),
            CONTACT_SENSOR_JSON_SCHEMA.to_owned(),
        );
        m.insert(
            "CLIMATE_SENSOR_JSON_SCHEMA".to_owned(),
            CLIMATE_SENSOR_JSON_SCHEMA.to_owned(),
        );
        m
    })
}

#[derive(Parser, Debug)]
#[command()]
struct Args {
    /// Subscription config path
    #[clap(long, default_value = "config/config.yaml")]
    config: PathBuf,

    /// Endpoints to connect to.
    #[clap(short = 'e', long)]
    connect: Vec<zenoh_config::EndPoint>,

    /// Endpoints to listen on.
    #[clap(long)]
    listen: Vec<zenoh_config::EndPoint>,

    /// foxglove bind address
    #[clap(long, default_value = "127.0.0.1:8765")]
    host: SocketAddr,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Args = Args::parse();
    setup_tracing()?;

    // read config
    let file = std::fs::File::open(&args.config)?;
    let config: foxglove_bridge::config::Configuration = serde_yaml::from_reader(file)?;

    // start foxglove server
    let server = foxglove_ws::FoxgloveWebSocket::new();
    tokio::spawn({
        let server = server.clone();
        async move { server.serve(args.host).await }
    });

    // configure zenoh
    // let mut zenoh_config = Config::default();
    // if !args.listen.is_empty() {
    //     zenoh_config.listen.endpoints = args.listen.clone();
    //     info!(listen_endpoints= ?zenoh_config.listen.endpoints, "Configured listening endpoints");
    // }
    // if !args.connect.is_empty() {
    //     zenoh_config.connect.endpoints = args.connect.clone();
    //     info!(connect_endpoints= ?zenoh_config.connect.endpoints, "Configured connect endpoints");
    // }

    let zenoh_config = config.zenoh.get_zenoh_config()?;

    let zenoh_session = zenoh::open(zenoh_config)
        .res()
        .await
        .map_err(FoxgloveBridgeError::ZenohError)?;
    let zenoh_session = zenoh_session.into_arc();
    info!("Started zenoh session");

    for proto_subscription in &config.protobuf_subscriptions {
        let message_descriptor = DESCRIPTOR_POOL
            .get_message_by_name(&proto_subscription.proto_type)
            .context("Failed to find protobuf message descriptor by name")?;

        start_proto_subscriber_from_descriptor(
            &proto_subscription.topic,
            zenoh_session.clone(),
            &server,
            &message_descriptor,
        )
        .await?;
    }

    for json_subscription in &config.json_subscriptions {
        let json_schema = if let Some(json_schema_name) = &json_subscription.json_schema_name {
            json_schema_table()
                .get(json_schema_name)
                .context("Failed to load json schema")?
        } else {
            GENERIC_JSON_SCHEMA
        };

        let latched = json_subscription.latched.unwrap_or(false);

        start_json_subscriber(
            &json_subscription.topic,
            zenoh_session.clone(),
            &server,
            &json_subscription.type_name,
            json_schema,
            latched,
        )
        .await?;
    }

    signal::ctrl_c().await?;
    info!("ctrl-c received, exiting");

    Ok(())
}

async fn start_proto_subscriber_from_descriptor(
    topic: &str,
    zenoh_session: Arc<Session>,
    foxglove_server: &FoxgloveWebSocket,
    protobuf_descriptor: &MessageDescriptor,
) -> anyhow::Result<()> {
    info!(topic, "Starting proto subscriber");
    let zenoh_subscriber = zenoh_session
        .declare_subscriber(topic)
        .res()
        .await
        .map_err(FoxgloveBridgeError::ZenohError)?;

    let foxglove_channel =
        create_publisher_for_protobuf_descriptor(protobuf_descriptor, foxglove_server, topic)
            .await?;

    tokio::spawn({
        let topic = topic.to_owned();
        async move {
            let mut message_counter = 0;
            loop {
                let res: anyhow::Result<()> = async {
                    let sample = zenoh_subscriber.recv_async().await?;
                    message_counter += 1;
                    let now = SystemTime::now();
                    let time_nanos = system_time_to_nanos(&now);
                    let payload: Vec<u8> = sample.value.try_into()?;
                    foxglove_channel.send(time_nanos, &payload).await?;

                    if message_counter % 20 == 0 {
                        info!(
                            topic,
                            message_counter, "{} sent {} messages", topic, message_counter
                        );
                    }
                    Ok(())
                }
                .await;
                if let Err(err) = res {
                    tracing::error!(topic, "Error receiving message: {}", err);
                }
            }
        }
    });
    Ok(())
}

const PROTOBUF_ENCODING: &str = "protobuf";

async fn create_publisher_for_protobuf_descriptor(
    protobuf_descriptor: &MessageDescriptor,
    foxglove_server: &FoxgloveWebSocket,
    topic: &str,
) -> anyhow::Result<Channel> {
    let protobuf_schema_data = protobuf_descriptor.parent_pool().encode_to_vec();
    foxglove_server
        .create_publisher(
            topic,
            PROTOBUF_ENCODING,
            protobuf_descriptor.full_name(),
            protobuf_schema_data,
            Some(PROTOBUF_ENCODING),
            false,
        )
        .await
}

const JSON_ENCODING: &str = "json";

async fn start_json_subscriber(
    topic: &str,
    zenoh_session: Arc<Session>,
    foxglove_server: &FoxgloveWebSocket,
    type_name: &str,
    json_shcema: &str,
    latched: bool,
) -> anyhow::Result<()> {
    info!(topic, "Starting json subscriber");
    let zenoh_subscriber = zenoh_session
        .declare_subscriber(topic)
        .res()
        .await
        .map_err(FoxgloveBridgeError::ZenohError)?;
    let foxglove_channel = foxglove_server
        .create_publisher(
            topic,
            JSON_ENCODING,
            type_name,
            json_shcema,
            Some("jsonschema"),
            latched,
        )
        .await?;

    tokio::spawn({
        let topic = topic.to_owned();
        async move {
            let mut message_counter = 0;
            loop {
                let res: anyhow::Result<()> = async {
                    let sample = zenoh_subscriber.recv_async().await?;
                    message_counter += 1;
                    let now = SystemTime::now();
                    let time_nanos = system_time_to_nanos(&now);

                    let payload = match &sample.encoding {
                        Encoding::Exact(KnownEncoding::TextPlain) => {
                            let payload: String = sample.value.try_into()?;
                            payload.as_bytes().to_vec()
                        }
                        Encoding::Exact(KnownEncoding::TextJson) => {
                            let payload: String = sample.value.try_into()?;
                            payload.as_bytes().to_vec()
                        }
                        Encoding::Exact(KnownEncoding::AppOctetStream) => {
                            let payload: Vec<u8> = sample.value.try_into()?;
                            payload
                        }
                        _ => {
                            tracing::error!(topic, "Unknown encoding: {:?}", sample.encoding);
                            panic!("Unknown encoding");
                        }
                    };

                    foxglove_channel.send(time_nanos, &payload).await?;

                    if message_counter % 20 == 0 {
                        info!(
                            topic,
                            message_counter, "{} sent {} messages", topic, message_counter
                        );
                    }
                    Ok(())
                }
                .await;
                if let Err(err) = res {
                    tracing::error!(topic, "Error receiving message: {}", err);
                }
            }
        }
    });
    Ok(())
}

#[allow(dead_code)]
const GENERIC_JSON_SCHEMA: &str = r#"
{
"title": "GenericJsonSchema",
"description": "Generic JSON Schema",
"type": "object",
"properties": {}
}
"#;

const IKEA_DIMMER_JSON_SCHEMA: &str = r#"
{
    "$schema": "http://json-schema.org/draft-04/schema#",
    "type": "object",
    "properties": {
      "action": {
        "type": "string"
      },
      "battery": {
        "type": "integer"
      },
      "brightness": {
        "type": "integer"
      },
      "linkquality": {
        "type": "integer"
      }
    },
    "required": [
      "action",
      "battery",
      "brightness",
      "linkquality"
    ]
}
"#;

const MOTION_SENSOR_JSON_SCHEMA: &str = r#"
{
    "$schema": "http://json-schema.org/draft-04/schema#",
    "type": "object",
    "properties": {
      "battery": {
        "type": "integer"
      },
      "battery_low": {
        "type": "boolean"
      },
      "linkquality": {
        "type": "integer"
      },
      "occupancy": {
        "type": "boolean"
      },
      "tamper": {
        "type": "boolean"
      },
      "voltage": {
        "type": "integer"
      }
    },
    "required": [
      "battery",
      "battery_low",
      "linkquality",
      "occupancy",
      "tamper",
      "voltage"
    ]
  }
"#;

const CONTACT_SENSOR_JSON_SCHEMA: &str = r#"
{
"$schema": "http://json-schema.org/draft-04/schema#",
"type": "object",
"properties": {
    "battery": {
    "type": "integer"
    },
    "battery_low": {
    "type": "boolean"
    },
    "contact": {
    "type": "boolean"
    },
    "linkquality": {
    "type": "integer"
    },
    "tamper": {
    "type": "boolean"
    },
    "voltage": {
    "type": "integer"
    }
},
"required": [
    "battery",
    "battery_low",
    "contact",
    "linkquality",
    "tamper",
    "voltage"
]
}
"#;

const CLIMATE_SENSOR_JSON_SCHEMA: &str = r#"
{
"$schema": "http://json-schema.org/draft-04/schema#",
"type": "object",
"properties": {
    "battery": {
    "type": "integer"
    },
    "humidity": {
    "type": "number"
    },
    "linkquality": {
    "type": "integer"
    },
    "temperature": {
    "type": "number"
    },
    "voltage": {
    "type": "integer"
    }
},
"required": [
    "battery",
    "humidity",
    "linkquality",
    "temperature",
    "voltage"
]
}
"#;
