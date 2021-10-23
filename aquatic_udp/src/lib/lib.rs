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
use privdrop::PrivDrop;

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

fn drop_privileges_after_socket_binding(
    config: &Config,
    num_bound_sockets: Arc<AtomicUsize>,
) -> anyhow::Result<()> {
    if config.privileges.drop_privileges {
        let mut counter = 0usize;

        loop {
            let sockets = num_bound_sockets.load(Ordering::SeqCst);

            if sockets == config.socket_workers {
                PrivDrop::default()
                    .chroot(config.privileges.chroot_path.clone())
                    .user(config.privileges.user.clone())
                    .apply()?;

                break;
            }

            ::std::thread::sleep(Duration::from_millis(10));

            counter += 1;

            if counter == 500 {
                panic!("Sockets didn't bind in time for privilege drop.");
            }
        }
    }

    Ok(())
}
