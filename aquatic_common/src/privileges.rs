use std::{
    path::PathBuf,
    sync::{Arc, Barrier},
};

use privdrop::PrivDrop;
use serde::Deserialize;

use aquatic_toml_config::TomlConfig;

#[derive(Clone, Debug, PartialEq, TomlConfig, Deserialize)]
#[serde(default)]
pub struct PrivilegeConfig {
    /// Chroot and switch group and user after binding to sockets
    pub drop_privileges: bool,
    /// Chroot to this path
    pub chroot_path: PathBuf,
    /// Group to switch to after chrooting
    pub group: String,
    /// User to switch to after chrooting
    pub user: String,
}

impl Default for PrivilegeConfig {
    fn default() -> Self {
        Self {
            drop_privileges: false,
            chroot_path: ".".into(),
            user: "nobody".to_string(),
            group: "nobody".to_string(),
        }
    }
}

#[derive(Clone)]
pub struct PrivilegeDropper {
    barrier: Arc<Barrier>,
    config: Arc<PrivilegeConfig>,
}

impl PrivilegeDropper {
    pub fn new(config: PrivilegeConfig, num_sockets: usize) -> Self {
        Self {
            barrier: Arc::new(Barrier::new(num_sockets)),
            config: Arc::new(config),
        }
    }

    pub fn after_socket_creation(&self) {
        if self.config.drop_privileges {
            if self.barrier.wait().is_leader() {
                PrivDrop::default()
                    .chroot(self.config.chroot_path.clone())
                    .user(self.config.user.clone())
                    .user(self.config.user.clone())
                    .apply()
                    .expect("drop privileges");
            }
        }
    }
}
