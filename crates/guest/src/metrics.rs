use std::{ops::Add, path::Path};

use anyhow::Result;
use krata::idm::protocol::{IdmMetricFormat, IdmMetricNode};
use sysinfo::Process;

pub struct MetricsCollector {}

impl MetricsCollector {
    pub fn new() -> Result<Self> {
        Ok(MetricsCollector {})
    }

    pub fn collect(&self) -> Result<IdmMetricNode> {
        let mut sysinfo = sysinfo::System::new();
        Ok(IdmMetricNode::structural(
            "guest",
            vec![
                self.collect_system(&mut sysinfo)?,
                self.collect_processes(&mut sysinfo)?,
            ],
        ))
    }

    fn collect_system(&self, sysinfo: &mut sysinfo::System) -> Result<IdmMetricNode> {
        sysinfo.refresh_memory();
        Ok(IdmMetricNode::structural(
            "system",
            vec![IdmMetricNode::structural(
                "memory",
                vec![
                    IdmMetricNode::value("total", sysinfo.total_memory(), IdmMetricFormat::Bytes),
                    IdmMetricNode::value("used", sysinfo.used_memory(), IdmMetricFormat::Bytes),
                    IdmMetricNode::value("free", sysinfo.free_memory(), IdmMetricFormat::Bytes),
                ],
            )],
        ))
    }

    fn collect_processes(&self, sysinfo: &mut sysinfo::System) -> Result<IdmMetricNode> {
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
        Ok(IdmMetricNode::structural("process", processes))
    }

    fn process_node(process: &Process) -> Result<IdmMetricNode> {
        let mut metrics = vec![];

        if let Some(parent) = process.parent() {
            metrics.push(IdmMetricNode::value(
                "parent",
                parent.as_u32() as u64,
                IdmMetricFormat::Integer,
            ));
        }

        if let Some(exe) = process.exe().and_then(path_as_str) {
            metrics.push(IdmMetricNode::raw_value("executable", exe));
        }

        if let Some(working_directory) = process.cwd().and_then(path_as_str) {
            metrics.push(IdmMetricNode::raw_value("cwd", working_directory));
        }

        let cmdline = process.cmd().to_vec();
        metrics.push(IdmMetricNode::raw_value("cmdline", cmdline));
        metrics.push(IdmMetricNode::structural(
            "memory",
            vec![
                IdmMetricNode::value("resident", process.memory(), IdmMetricFormat::Bytes),
                IdmMetricNode::value("virtual", process.virtual_memory(), IdmMetricFormat::Bytes),
            ],
        ));

        metrics.push(IdmMetricNode::value(
            "lifetime",
            process.run_time(),
            IdmMetricFormat::DurationSeconds,
        ));
        metrics.push(IdmMetricNode::value(
            "uid",
            process.user_id().map(|x| (*x).add(0)).unwrap_or(0) as f64,
            IdmMetricFormat::Integer,
        ));
        metrics.push(IdmMetricNode::value(
            "gid",
            process.group_id().map(|x| (*x).add(0)).unwrap_or(0) as f64,
            IdmMetricFormat::Integer,
        ));
        metrics.push(IdmMetricNode::value(
            "euid",
            process
                .effective_user_id()
                .map(|x| (*x).add(0))
                .unwrap_or(0) as f64,
            IdmMetricFormat::Integer,
        ));
        metrics.push(IdmMetricNode::value(
            "egid",
            process.effective_group_id().map(|x| x.add(0)).unwrap_or(0) as f64,
            IdmMetricFormat::Integer,
        ));

        Ok(IdmMetricNode::structural(
            process.pid().to_string(),
            metrics,
        ))
    }
}

fn path_as_str(path: &Path) -> Option<String> {
    String::from_utf8(path.as_os_str().as_encoded_bytes().to_vec()).ok()
}
