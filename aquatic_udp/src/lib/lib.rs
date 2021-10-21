use std::sync::Arc;

use aquatic_common::access_list::{AccessListArcSwap, AccessListMode, AccessListQuery};

pub mod common;
pub mod config;
#[cfg(all(feature = "with-glommio", target_os = "linux"))]
pub mod glommio;
pub mod mio;

use config::Config;

pub const APP_NAME: &str = "aquatic_udp: UDP BitTorrent tracker";

pub fn run(config: Config) -> ::anyhow::Result<()> {
    cfg_if::cfg_if! {
        if #[cfg(all(feature = "with-glommio", target_os = "linux"))] {
            glommio::run(config)
        } else {
            mio::run(config)
        }
    }
}

pub fn update_access_list(config: &Config, access_list: &Arc<AccessListArcSwap>) {
    match config.access_list.mode {
        AccessListMode::White | AccessListMode::Black => {
            if let Err(err) = access_list.update_from_path(&config.access_list.path) {
                ::log::error!("Update access list from path: {:?}", err);
            }
        }
        AccessListMode::Off => {}
    }
}
