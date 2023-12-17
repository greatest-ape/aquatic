use std::{fmt::Display, ops::Range, thread::available_parallelism};

use itertools::Itertools;

#[derive(Debug, Clone)]
pub struct TaskSetCpuList(pub Vec<TaskSetCpuIndicator>);

impl TaskSetCpuList {
    pub fn as_cpu_list(&self) -> String {
        let indicator = self.0.iter().map(|indicator| match indicator {
            TaskSetCpuIndicator::Single(i) => i.to_string(),
            TaskSetCpuIndicator::Range(range) => {
                format!(
                    "{}-{}",
                    range.start,
                    range.clone().into_iter().last().unwrap()
                )
            }
        });

        Itertools::intersperse_with(indicator, || ",".to_string())
            .into_iter()
            .collect()
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
                CpuMode::Split => {
                    let middle = available_parallelism / 2;

                    let range_a = 0..(middle.min(requested_cpus));
                    let range_b = middle..(available_parallelism.min(middle + requested_cpus));

                    Self(vec![
                        range_a.try_into().unwrap(),
                        range_b.try_into().unwrap(),
                    ])
                }
                CpuMode::All => {
                    let range = 0..(available_parallelism.min(requested_cpus));

                    Self(vec![range.try_into().unwrap()])
                }
            },
            CpuDirection::Desc => match mode {
                CpuMode::Split => {
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
                CpuMode::All => {
                    let range =
                        available_parallelism.saturating_sub(requested_cpus)..available_parallelism;

                    Self(vec![range.try_into().unwrap()])
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
    Split,
    All,
}

impl Display for CpuMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::All => f.write_str("all"),
            Self::Split => f.write_str("split"),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum CpuDirection {
    Asc,
    Desc,
}

pub fn simple_load_test_runs(cpu_mode: CpuMode, workers: &[usize]) -> Vec<(usize, TaskSetCpuList)> {
    workers
        .into_iter()
        .copied()
        .map(|workers| {
            (
                workers,
                TaskSetCpuList::new(cpu_mode, CpuDirection::Desc, workers).unwrap(),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_set_cpu_list_split_asc() {
        let f = TaskSetCpuList::new_with_available_parallelism;

        assert_eq!(
            f(8, CpuMode::Split, CpuDirection::Asc, 1).as_cpu_list(),
            "0,4"
        );
        assert_eq!(
            f(8, CpuMode::Split, CpuDirection::Asc, 2).as_cpu_list(),
            "0-1,4-5"
        );
        assert_eq!(
            f(8, CpuMode::Split, CpuDirection::Asc, 4).as_cpu_list(),
            "0-3,4-7"
        );
        assert_eq!(
            f(8, CpuMode::Split, CpuDirection::Asc, 8).as_cpu_list(),
            "0-3,4-7"
        );
        assert_eq!(
            f(8, CpuMode::Split, CpuDirection::Asc, 9).as_cpu_list(),
            "0-3,4-7"
        );
    }

    #[test]
    fn test_task_set_cpu_list_split_desc() {
        let f = TaskSetCpuList::new_with_available_parallelism;

        assert_eq!(
            f(8, CpuMode::Split, CpuDirection::Desc, 1).as_cpu_list(),
            "3,7"
        );
        assert_eq!(
            f(8, CpuMode::Split, CpuDirection::Desc, 2).as_cpu_list(),
            "2-3,6-7"
        );
        assert_eq!(
            f(8, CpuMode::Split, CpuDirection::Desc, 4).as_cpu_list(),
            "0-3,4-7"
        );
        assert_eq!(
            f(8, CpuMode::Split, CpuDirection::Desc, 8).as_cpu_list(),
            "0-3,4-7"
        );
        assert_eq!(
            f(8, CpuMode::Split, CpuDirection::Desc, 9).as_cpu_list(),
            "0-3,4-7"
        );
    }

    #[test]
    fn test_task_set_cpu_list_all_asc() {
        let f = TaskSetCpuList::new_with_available_parallelism;

        assert_eq!(f(8, CpuMode::All, CpuDirection::Asc, 1).as_cpu_list(), "0");
        assert_eq!(
            f(8, CpuMode::All, CpuDirection::Asc, 2).as_cpu_list(),
            "0-1"
        );
        assert_eq!(
            f(8, CpuMode::All, CpuDirection::Asc, 4).as_cpu_list(),
            "0-3"
        );
        assert_eq!(
            f(8, CpuMode::All, CpuDirection::Asc, 8).as_cpu_list(),
            "0-7"
        );
        assert_eq!(
            f(8, CpuMode::All, CpuDirection::Asc, 9).as_cpu_list(),
            "0-7"
        );
    }

    #[test]
    fn test_task_set_cpu_list_all_desc() {
        let f = TaskSetCpuList::new_with_available_parallelism;

        assert_eq!(f(8, CpuMode::All, CpuDirection::Desc, 1).as_cpu_list(), "7");
        assert_eq!(
            f(8, CpuMode::All, CpuDirection::Desc, 2).as_cpu_list(),
            "6-7"
        );
        assert_eq!(
            f(8, CpuMode::All, CpuDirection::Desc, 4).as_cpu_list(),
            "4-7"
        );
        assert_eq!(
            f(8, CpuMode::All, CpuDirection::Desc, 8).as_cpu_list(),
            "0-7"
        );
        assert_eq!(
            f(8, CpuMode::All, CpuDirection::Desc, 9).as_cpu_list(),
            "0-7"
        );
    }
}
