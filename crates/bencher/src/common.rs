use std::{fmt::Display, ops::Range, thread::available_parallelism};

use itertools::Itertools;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, clap::ValueEnum)]
pub enum Priority {
    Low,
    Medium,
    High,
}

impl Display for Priority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Low => f.write_str("low"),
            Self::Medium => f.write_str("medium"),
            Self::High => f.write_str("high"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TaskSetCpuList(pub Vec<TaskSetCpuIndicator>);

impl TaskSetCpuList {
    pub fn as_cpu_list(&self) -> String {
        let indicator = self.0.iter().map(|indicator| match indicator {
            TaskSetCpuIndicator::Single(i) => i.to_string(),
            TaskSetCpuIndicator::Range(range) => {
                format!("{}-{}", range.start, range.clone().last().unwrap())
            }
        });

        Itertools::intersperse_with(indicator, || ",".to_string()).collect()
    }

    pub fn new(
        mode: CpuMode,
        direction: CpuDirection,
        requested_cpus: usize,
    ) -> anyhow::Result<Self> {
        let available_parallelism: usize = available_parallelism()?.into();

        Ok(Self::new_with_available_parallelism(
            available_parallelism,
            mode,
            direction,
            requested_cpus,
        ))
    }

    fn new_with_available_parallelism(
        available_parallelism: usize,
        mode: CpuMode,
        direction: CpuDirection,
        requested_cpus: usize,
    ) -> Self {
        match direction {
            CpuDirection::Asc => match mode {
                CpuMode::Subsequent => {
                    let range = 0..(available_parallelism.min(requested_cpus));

                    Self(vec![range.try_into().unwrap()])
                }
                CpuMode::SplitPairs => {
                    let middle = available_parallelism / 2;

                    let range_a = 0..(middle.min(requested_cpus));
                    let range_b = middle..(available_parallelism.min(middle + requested_cpus));

                    Self(vec![
                        range_a.try_into().unwrap(),
                        range_b.try_into().unwrap(),
                    ])
                }
                CpuMode::SubsequentPairs => {
                    let range = 0..(available_parallelism.min(requested_cpus * 2));

                    Self(vec![range.try_into().unwrap()])
                }
                CpuMode::SubsequentOnePerPair => {
                    let range = 0..(available_parallelism.min(requested_cpus * 2));

                    Self(
                        range
                            .chunks(2)
                            .into_iter()
                            .map(|mut chunk| TaskSetCpuIndicator::Single(chunk.next().unwrap()))
                            .collect(),
                    )
                }
            },
            CpuDirection::Desc => match mode {
                CpuMode::Subsequent => {
                    let range =
                        available_parallelism.saturating_sub(requested_cpus)..available_parallelism;

                    Self(vec![range.try_into().unwrap()])
                }
                CpuMode::SplitPairs => {
                    let middle = available_parallelism / 2;

                    let range_a = middle.saturating_sub(requested_cpus)..middle;
                    let range_b = available_parallelism
                        .saturating_sub(requested_cpus)
                        .max(middle)..available_parallelism;

                    Self(vec![
                        range_a.try_into().unwrap(),
                        range_b.try_into().unwrap(),
                    ])
                }
                CpuMode::SubsequentPairs => {
                    let range = available_parallelism.saturating_sub(requested_cpus * 2)
                        ..available_parallelism;

                    Self(vec![range.try_into().unwrap()])
                }
                CpuMode::SubsequentOnePerPair => {
                    let range = available_parallelism.saturating_sub(requested_cpus * 2)
                        ..available_parallelism;

                    Self(
                        range
                            .chunks(2)
                            .into_iter()
                            .map(|mut chunk| TaskSetCpuIndicator::Single(chunk.next().unwrap()))
                            .collect(),
                    )
                }
            },
        }
    }
}

impl TryFrom<Vec<Range<usize>>> for TaskSetCpuList {
    type Error = String;

    fn try_from(value: Vec<Range<usize>>) -> Result<Self, Self::Error> {
        let mut output = Vec::new();

        for range in value {
            output.push(range.try_into()?);
        }

        Ok(Self(output))
    }
}

#[derive(Debug, Clone)]
pub enum TaskSetCpuIndicator {
    Single(usize),
    Range(Range<usize>),
}

impl TryFrom<Range<usize>> for TaskSetCpuIndicator {
    type Error = String;

    fn try_from(value: Range<usize>) -> Result<Self, Self::Error> {
        match value.len() {
            0 => Err("Empty ranges not supported".into()),
            1 => Ok(TaskSetCpuIndicator::Single(value.start)),
            _ => Ok(TaskSetCpuIndicator::Range(value)),
        }
    }
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum CpuMode {
    /// Suitable for bare-metal machines without hyperthreads/SMT.
    ///
    /// For 8 vCPU processor, uses vCPU groups 0, 1, 2, 3, 4, 5, 6 and 7
    Subsequent,
    /// Suitable for bare-metal machines with hyperthreads/SMT.
    ///
    /// For 8 vCPU processor, uses vCPU groups 0 & 4, 1 & 5, 2 & 6 and 3 & 7
    SplitPairs,
    /// For 8 vCPU processor, uses vCPU groups 0 & 1, 2 & 3, 4 & 5 and 6 & 7
    SubsequentPairs,
    /// Suitable for somewhat fairly comparing trackers on Hetzner virtual
    /// machines. Since in-VM hyperthreads aren't really hyperthreads,
    /// enabling them causes unpredictable performance.
    ///
    /// For 8 vCPU processor, uses vCPU groups 0, 2, 4 and 6
    SubsequentOnePerPair,
}

impl Display for CpuMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Subsequent => f.write_str("subsequent"),
            Self::SplitPairs => f.write_str("split-pairs"),
            Self::SubsequentPairs => f.write_str("subsequent-pairs"),
            Self::SubsequentOnePerPair => f.write_str("subsequent-one-per-pair"),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum CpuDirection {
    Asc,
    Desc,
}

pub fn simple_load_test_runs(
    cpu_mode: CpuMode,
    workers: &[(usize, Priority)],
) -> Vec<(usize, Priority, TaskSetCpuList)> {
    workers
        .iter()
        .copied()
        .map(|(workers, priority)| {
            (
                workers,
                priority,
                TaskSetCpuList::new(cpu_mode, CpuDirection::Desc, workers).unwrap(),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_set_cpu_list_split_pairs_asc() {
        let f = TaskSetCpuList::new_with_available_parallelism;

        let mode = CpuMode::SplitPairs;
        let direction = CpuDirection::Asc;

        assert_eq!(f(8, mode, direction, 1).as_cpu_list(), "0,4");
        assert_eq!(f(8, mode, direction, 2).as_cpu_list(), "0-1,4-5");
        assert_eq!(f(8, mode, direction, 4).as_cpu_list(), "0-3,4-7");
        assert_eq!(f(8, mode, direction, 8).as_cpu_list(), "0-3,4-7");
        assert_eq!(f(8, mode, direction, 9).as_cpu_list(), "0-3,4-7");
    }

    #[test]
    fn test_task_set_cpu_list_split_pairs_desc() {
        let f = TaskSetCpuList::new_with_available_parallelism;

        let mode = CpuMode::SplitPairs;
        let direction = CpuDirection::Desc;

        assert_eq!(f(8, mode, direction, 1).as_cpu_list(), "3,7");
        assert_eq!(f(8, mode, direction, 2).as_cpu_list(), "2-3,6-7");
        assert_eq!(f(8, mode, direction, 4).as_cpu_list(), "0-3,4-7");
        assert_eq!(f(8, mode, direction, 8).as_cpu_list(), "0-3,4-7");
        assert_eq!(f(8, mode, direction, 9).as_cpu_list(), "0-3,4-7");
    }

    #[test]
    fn test_task_set_cpu_list_subsequent_asc() {
        let f = TaskSetCpuList::new_with_available_parallelism;

        let mode = CpuMode::Subsequent;
        let direction = CpuDirection::Asc;

        assert_eq!(f(8, mode, direction, 1).as_cpu_list(), "0");
        assert_eq!(f(8, mode, direction, 2).as_cpu_list(), "0-1");
        assert_eq!(f(8, mode, direction, 4).as_cpu_list(), "0-3");
        assert_eq!(f(8, mode, direction, 8).as_cpu_list(), "0-7");
        assert_eq!(f(8, mode, direction, 9).as_cpu_list(), "0-7");
    }

    #[test]
    fn test_task_set_cpu_list_subsequent_desc() {
        let f = TaskSetCpuList::new_with_available_parallelism;

        let mode = CpuMode::Subsequent;
        let direction = CpuDirection::Desc;

        assert_eq!(f(8, mode, direction, 1).as_cpu_list(), "7");
        assert_eq!(f(8, mode, direction, 2).as_cpu_list(), "6-7");
        assert_eq!(f(8, mode, direction, 4).as_cpu_list(), "4-7");
        assert_eq!(f(8, mode, direction, 8).as_cpu_list(), "0-7");
        assert_eq!(f(8, mode, direction, 9).as_cpu_list(), "0-7");
    }

    #[test]
    fn test_task_set_cpu_list_subsequent_pairs_asc() {
        let f = TaskSetCpuList::new_with_available_parallelism;
        let mode = CpuMode::SubsequentPairs;
        let direction = CpuDirection::Asc;

        assert_eq!(f(8, mode, direction, 1).as_cpu_list(), "0-1");
        assert_eq!(f(8, mode, direction, 2).as_cpu_list(), "0-3");
        assert_eq!(f(8, mode, direction, 4).as_cpu_list(), "0-7");
        assert_eq!(f(8, mode, direction, 8).as_cpu_list(), "0-7");
        assert_eq!(f(8, mode, direction, 9).as_cpu_list(), "0-7");
    }

    #[test]
    fn test_task_set_cpu_list_subsequent_pairs_desc() {
        let f = TaskSetCpuList::new_with_available_parallelism;

        let mode = CpuMode::SubsequentPairs;
        let direction = CpuDirection::Desc;

        assert_eq!(f(8, mode, direction, 1).as_cpu_list(), "6-7");
        assert_eq!(f(8, mode, direction, 2).as_cpu_list(), "4-7");
        assert_eq!(f(8, mode, direction, 4).as_cpu_list(), "0-7");
        assert_eq!(f(8, mode, direction, 8).as_cpu_list(), "0-7");
        assert_eq!(f(8, mode, direction, 9).as_cpu_list(), "0-7");
    }

    #[test]
    fn test_task_set_cpu_list_subsequent_one_per_pair_asc() {
        let f = TaskSetCpuList::new_with_available_parallelism;

        let mode = CpuMode::SubsequentOnePerPair;
        let direction = CpuDirection::Asc;

        assert_eq!(f(8, mode, direction, 1).as_cpu_list(), "0");
        assert_eq!(f(8, mode, direction, 2).as_cpu_list(), "0,2");
        assert_eq!(f(8, mode, direction, 4).as_cpu_list(), "0,2,4,6");
        assert_eq!(f(8, mode, direction, 8).as_cpu_list(), "0,2,4,6");
        assert_eq!(f(8, mode, direction, 9).as_cpu_list(), "0,2,4,6");
    }

    #[test]
    fn test_task_set_cpu_list_subsequent_one_per_pair_desc() {
        let f = TaskSetCpuList::new_with_available_parallelism;

        let mode = CpuMode::SubsequentOnePerPair;
        let direction = CpuDirection::Desc;

        assert_eq!(f(8, mode, direction, 1).as_cpu_list(), "6");
        assert_eq!(f(8, mode, direction, 2).as_cpu_list(), "4,6");
        assert_eq!(f(8, mode, direction, 4).as_cpu_list(), "0,2,4,6");
        assert_eq!(f(8, mode, direction, 8).as_cpu_list(), "0,2,4,6");
        assert_eq!(f(8, mode, direction, 9).as_cpu_list(), "0,2,4,6");
    }
}
