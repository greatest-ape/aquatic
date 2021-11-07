use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CpuPinningMode {
    Ascending,
    Descending,
}

impl Default for CpuPinningMode {
    fn default() -> Self {
        Self::Ascending
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CpuPinningConfig {
    pub active: bool,
    pub mode: CpuPinningMode,
    pub offset: usize,
    pub multiple: usize,
}

impl Default for CpuPinningConfig {
    fn default() -> Self {
        Self {
            active: false,
            mode: Default::default(),
            offset: 0,
            multiple: 1,
        }
    }
}

impl CpuPinningConfig {
    pub fn default_for_load_test() -> Self {
        Self {
            mode: CpuPinningMode::Descending,
            ..Default::default()
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum WorkerIndex {
    SocketWorker(usize),
    RequestWorker(usize),
    Other,
}

impl WorkerIndex {
    pub fn get_cpu_index(self, config: &CpuPinningConfig, socket_workers: usize) -> usize {
        let index = match self {
            Self::Other => config.offset,
            Self::SocketWorker(index) => config.multiple * (config.offset + 1 + index),
            Self::RequestWorker(index) => {
                config.multiple * (config.offset + 1 + socket_workers + index)
            }
        };

        let index = match config.mode {
            CpuPinningMode::Ascending => index,
            CpuPinningMode::Descending => {
                let max = core_affinity::get_core_ids()
                    .map(|ids| ids.iter().map(|id| id.id).max())
                    .flatten()
                    .unwrap_or(0);

                max - index
            }
        };

        ::log::info!("Calculated CPU pin index {} for {:?}", index, self);

        index
    }
}

pub fn pin_current_if_configured_to(
    config: &CpuPinningConfig,
    socket_workers: usize,
    worker_index: WorkerIndex,
) {
    if config.active {
        core_affinity::set_for_current(core_affinity::CoreId {
            id: worker_index.get_cpu_index(config, socket_workers),
        });
    }
}
