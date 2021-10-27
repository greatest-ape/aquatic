use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

use cfg_if::cfg_if;

pub mod common;
pub mod config;
#[cfg(all(feature = "with-glommio", target_os = "linux"))]
pub mod glommio;
#[cfg(feature = "with-mio")]
pub mod mio;

use config::Config;

pub const APP_NAME: &str = "aquatic_udp: UDP BitTorrent tracker";

pub fn run(config: Config) -> ::anyhow::Result<()> {
    cfg_if! {
        if #[cfg(all(feature = "with-glommio", target_os = "linux"))] {
            glommio::run(config)
        } else {
            mio::run(config)
        }
    }
}
