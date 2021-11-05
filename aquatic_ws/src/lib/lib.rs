use aquatic_common::access_list::update_access_list;
use signal_hook::{consts::SIGUSR1, iterator::Signals};

use crate::config::Config;

pub mod common;
pub mod config;
pub mod glommio;
pub mod mio;

pub const APP_NAME: &str = "aquatic_ws: WebTorrent tracker";

pub fn run(config: Config) -> ::anyhow::Result<()> {
    if config.cpu_pinning.active {
        core_affinity::set_for_current(core_affinity::CoreId {
            id: config.cpu_pinning.offset,
        });
    }

    let state = glommio::common::State::default();

    update_access_list(&config.access_list, &state.access_list)?;

    let mut signals = Signals::new(::std::iter::once(SIGUSR1))?;

    {
        let config = config.clone();
        let state = state.clone();

        ::std::thread::spawn(move || glommio::run_inner(config, state));
    }

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
