use aquatic_common::access_list::update_access_list;
#[cfg(feature = "cpu-pinning")]
use aquatic_common::cpu_pinning::{pin_current_if_configured_to, WorkerIndex};
use cfg_if::cfg_if;
use signal_hook::{consts::SIGUSR1, iterator::Signals};

use crate::config::Config;

pub mod common;
pub mod config;
#[cfg(feature = "with-glommio")]
pub mod glommio;
#[cfg(feature = "with-mio")]
pub mod mio;

pub const APP_NAME: &str = "aquatic_ws: WebTorrent tracker";

pub fn run(config: Config) -> ::anyhow::Result<()> {
    cfg_if!(
        if #[cfg(feature = "with-glommio")] {
            let state = glommio::common::State::default();
        } else {
            let state = mio::common::State::default();
        }
    );

    update_access_list(&config.access_list, &state.access_list)?;

    let mut signals = Signals::new(::std::iter::once(SIGUSR1))?;

    {
        let config = config.clone();
        let state = state.clone();

        cfg_if!(
            if #[cfg(feature = "with-glommio")] {
                ::std::thread::spawn(move || glommio::run(config, state));
            } else {
                ::std::thread::spawn(move || mio::run(config, state));
            }
        );
    }

    #[cfg(feature = "cpu-pinning")]
    pin_current_if_configured_to(
        &config.cpu_pinning,
        config.socket_workers,
        WorkerIndex::Other,
    );

    for signal in &mut signals {
        match signal {
            SIGUSR1 => {
                let _ = update_access_list(&config.access_list, &state.access_list);
            }
            _ => unreachable!(),
        }
    }

    Ok(())
}
