pub mod config;

use anyhow::Context;
use std::result::Result;
use thiserror::Error;
use tracing::{dispatcher, Dispatch};
use tracing_subscriber::{prelude::__tracing_subscriber_SubscriberExt, EnvFilter, Registry};

pub fn setup_tracing() -> anyhow::Result<()> {
    let filter = EnvFilter::builder()
        .with_default_directive(tracing_subscriber::filter::LevelFilter::INFO.into())
        .parse("")?;

    let subscriber = Registry::default()
        .with(filter)
        .with(tracing_logfmt::layer());
    dispatcher::set_global_default(Dispatch::new(subscriber))
        .context("Global logger has already been set!")?;
    Ok(())
}

#[derive(Error, Debug)]
pub enum FoxgloveBridgeError {
    #[error("Zenoh error {0:?}")]
    ZenohError(#[from] zenoh::Error),
}

pub type FoxgloveBridgeResult<T> = Result<T, FoxgloveBridgeError>;
