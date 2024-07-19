use std::{ops::Add, path::Path};

use anyhow::Result;
use krata::idm::internal::{MetricFormat, MetricNode};
use sysinfo::Process;

pub struct MetricsCollector {}

impl MetricsCollector {
    pub fn new() -> Result<Self> {
        Ok(MetricsCollector {})
    }

    pub fn collect(&self) -> Result<MetricNode> {
        let mut sysinfo = sysinfo::System::new();
        Ok(MetricNode::structural(
            "zone",
            vec![
                self.collect_system(&mut sysinfo)?,
                self.collect_processes(&mut sysinfo)?,
            ],
        ))
    }

    fn collect_system(&self, sysinfo: &mut sysinfo::System) -> Result<MetricNode> {
        sysinfo.refresh_memory();
        Ok(MetricNode::structural(
            "system",
            vec![MetricNode::structural(
                "memory",
                vec![
                    MetricNode::value("total", sysinfo.total_memory(), MetricFormat::Bytes),
                    MetricNode::value("used", sysinfo.used_memory(), MetricFormat::Bytes),
                    MetricNode::value("free", sysinfo.free_memory(), MetricFormat::Bytes),
                ],
            )],
        ))
    }

    fn collect_processes(&self, sysinfo: &mut sysinfo::System) -> Result<MetricNode> {
        sysinfo.refresh_processes();
        let mut processes = Vec::new();
        let mut sysinfo_processes = sysinfo.processes().values().collect::<Vec<_>>();
        sysinfo_processes.sort_by_key(|x| x.pid());
        for process in sysinfo_processes {
            if process.thread_kind().is_some() {
                continue;
            }
            processes.push(MetricsCollector::process_node(process)?);
        }
        Ok(MetricNode::structural("process", processes))
    }

    fn process_node(process: &Process) -> Result<MetricNode> {
        let mut metrics = vec![];

        if let Some(parent) = process.parent() {
            metrics.push(MetricNode::value(
                "parent",
                parent.as_u32() as u64,
                MetricFormat::Integer,
            ));
        }

        if let Some(exe) = process.exe().and_then(path_as_str) {
            metrics.push(MetricNode::raw_value("executable", exe));
        }

        if let Some(working_directory) = process.cwd().and_then(path_as_str) {
            metrics.push(MetricNode::raw_value("cwd", working_directory));
        }

        let cmdline = process.cmd().to_vec();
        metrics.push(MetricNode::raw_value("cmdline", cmdline));
        metrics.push(MetricNode::structural(
            "memory",
            vec![
                MetricNode::value("resident", process.memory(), MetricFormat::Bytes),
                MetricNode::value("virtual", process.virtual_memory(), MetricFormat::Bytes),
            ],
        ));

        metrics.push(MetricNode::value(
            "lifetime",
            process.run_time(),
            MetricFormat::DurationSeconds,
        ));
        metrics.push(MetricNode::value(
            "uid",
            process.user_id().map(|x| (*x).add(0)).unwrap_or(0) as f64,
            MetricFormat::Integer,
        ));
        metrics.push(MetricNode::value(
            "gid",
            process.group_id().map(|x| (*x).add(0)).unwrap_or(0) as f64,
            MetricFormat::Integer,
        ));
        metrics.push(MetricNode::value(
            "euid",
            process
                .effective_user_id()
                .map(|x| (*x).add(0))
                .unwrap_or(0) as f64,
            MetricFormat::Integer,
        ));
        metrics.push(MetricNode::value(
            "egid",
            process.effective_group_id().map(|x| x.add(0)).unwrap_or(0) as f64,
            MetricFormat::Integer,
        ));

        Ok(MetricNode::structural(process.pid().to_string(), metrics))
    }
}

fn path_as_str(path: &Path) -> Option<String> {
    String::from_utf8(path.as_os_str().as_encoded_bytes().to_vec()).ok()
}
