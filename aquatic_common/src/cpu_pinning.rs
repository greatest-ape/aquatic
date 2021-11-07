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
    pub virtual_per_physical_cpu: usize,
    pub offset_cpus: usize,
}

impl Default for CpuPinningConfig {
    fn default() -> Self {
        Self {
            active: false,
            mode: Default::default(),
            virtual_per_physical_cpu: 2,
            offset_cpus: 0,
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
    fn get_cpu_indices(self, config: &CpuPinningConfig, socket_workers: usize) -> Vec<usize> {
        let offset = match self {
            Self::Other => config.virtual_per_physical_cpu * config.offset_cpus,
            Self::SocketWorker(index) => {
                config.virtual_per_physical_cpu * (config.offset_cpus + 1 + index)
            }
            Self::RequestWorker(index) => {
                config.virtual_per_physical_cpu * (config.offset_cpus + 1 + socket_workers + index)
            }
        };

        let virtual_cpus = (0..config.virtual_per_physical_cpu).map(|i| offset + i);

        let virtual_cpus: Vec<usize> = match config.mode {
            CpuPinningMode::Ascending => virtual_cpus.collect(),
            CpuPinningMode::Descending => {
                let max_index = affinity::get_core_num() - 1;

                virtual_cpus
                    .map(|i| max_index.checked_sub(i).unwrap_or(0))
                    .collect()
            }
        };

        ::log::info!(
            "Calculated virtual CPU pin indices {:?} for {:?}",
            virtual_cpus,
            self
        );

        virtual_cpus
    }
}

/// Note: don't call this when affinities were already set in the current or in
/// a parent thread. Doing so limits the number of cores that are seen and
/// messes up setting affinities.
pub fn pin_current_if_configured_to(
    config: &CpuPinningConfig,
    socket_workers: usize,
    worker_index: WorkerIndex,
) {
    if config.active {
        let indices = worker_index.get_cpu_indices(config, socket_workers);

        if let Err(err) = affinity::set_thread_affinity(indices.clone()) {
            ::log::error!(
                "Failed setting thread affinities {:?} for {:?}: {:#?}",
                indices,
                worker_index,
                err
            );
        }
    }
}
