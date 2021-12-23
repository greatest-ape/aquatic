use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

use privdrop::PrivDrop;
use serde::{Deserialize};
use toml_config::TomlConfig;

#[derive(Clone, Debug, PartialEq, TomlConfig, Deserialize)]
#[serde(default)]
pub struct PrivilegeConfig {
    /// Chroot and switch user after binding to sockets
    pub drop_privileges: bool,
    /// Chroot to this path
    pub chroot_path: String,
    /// User to switch to after chrooting
    pub user: String,
}

impl Default for PrivilegeConfig {
    fn default() -> Self {
        Self {
            drop_privileges: false,
            chroot_path: ".".to_string(),
            user: "nobody".to_string(),
        }
    }
}

pub fn drop_privileges_after_socket_binding(
    config: &PrivilegeConfig,
    num_bound_sockets: Arc<AtomicUsize>,
    target_num: usize,
) -> anyhow::Result<()> {
    if config.drop_privileges {
        let mut counter = 0usize;

        loop {
            let num_bound = num_bound_sockets.load(Ordering::SeqCst);

            if num_bound == target_num {
                PrivDrop::default()
                    .chroot(config.chroot_path.clone())
                    .user(config.user.clone())
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
