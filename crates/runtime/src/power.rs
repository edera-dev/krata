use anyhow::Result;
use indexmap::IndexMap;
use xencall::sys::{CpuId, SysctlCputopo};

use crate::RuntimeContext;

#[derive(Clone)]
pub struct PowerManagementContext {
    pub context: RuntimeContext,
}

#[derive(Clone, Copy, Debug)]
pub enum CpuClass {
    Standard,
    Performance,
    Efficiency,
}

#[derive(Clone, Copy, Debug)]
pub struct CpuTopologyInfo {
    pub core: u32,
    pub socket: u32,
    pub node: u32,
    pub thread: u32,
    pub class: CpuClass,
}

fn labelled_topo(input: &[SysctlCputopo]) -> Vec<CpuTopologyInfo> {
    let mut cores: IndexMap<(u32, u32, u32), Vec<CpuTopologyInfo>> = IndexMap::new();
    let mut pe_cores = false;
    let mut last: Option<SysctlCputopo> = None;

    for item in input {
        if cores.is_empty() {
            cores.insert((item.core, item.socket, item.node), vec![
                CpuTopologyInfo {
                    core: item.core,
                    socket: item.socket,
                    thread: 0,
                    node: item.node,
                    class: CpuClass::Standard,
                }
            ]);
            continue;
        }
        
        if last.map(|last| item.core == last.core + 4).unwrap_or(false) { // detect if performance cores seem to be kicking in.
            if let Some(last) = last {
                if let Some(list) = cores.get_mut(&(last.core, last.socket, last.node)) {
                    for other in list {
                        other.class = CpuClass::Performance;
                    }
                }
            }
            let list = cores.entry((item.core, item.socket, item.node)).or_default();
            for old in &mut *list {
                old.class = CpuClass::Performance;
            }
            list.push(CpuTopologyInfo {
                core: item.core,
                socket: item.socket,
                thread: 0,
                node: item.node,
                class: CpuClass::Performance,
            });
            pe_cores = true;
        } else if pe_cores && last.map(|last| item.core == last.core + 1).unwrap_or(false) { // detect efficiency cores if P/E cores are in use.
            let list =  cores.entry((item.core, item.socket, item.node)).or_default();
            list.push(CpuTopologyInfo {
                core: item.core,
                socket: item.socket,
                thread: 0,
                node: item.node,
                class: CpuClass::Efficiency,
            });
        } else {
            let list =  cores.entry((item.core, item.socket, item.node)).or_default();
            if list.is_empty() {
                list.push(CpuTopologyInfo {
                    core: item.core,
                    socket: item.socket,
                    thread: 0,
                    node: item.node,
                    class: CpuClass::Standard,
                });
            } else {
                list.push(CpuTopologyInfo {
                    core: item.core,
                    socket: item.socket,
                    thread: 0,
                    node: item.node,
                    class: list.first().map(|first| first.class).unwrap_or(CpuClass::Standard),
                });
            }
        }
        last = Some(item.clone());
    }

    for threads in cores.values_mut() {
        for (index, thread) in threads.iter_mut().enumerate() {
            thread.thread = index as u32;
        }
    }
    
    cores.into_values().into_iter().flatten().collect::<Vec<_>>()
}

impl PowerManagementContext {
    /// Get the CPU topology, with SMT awareness.
    /// Also translates Intel p-core/e-core nonsense: non-sequential core identifiers
    /// are treated as p-cores, while e-cores behave as standard cores.
    /// If there is a p-core/e-core split, then CPU class will be defined as
    /// `CpuClass::Performance` or `CpuClass::Efficiency`, else `CpuClass::Standard`.
    pub async fn cpu_topology(self) -> Result<Vec<CpuTopologyInfo>> {
        let xentopo = self.context.xen.call.cpu_topology().await?;
        let logicaltopo = labelled_topo(&xentopo);
        Ok(logicaltopo)
    }

    /// Enable or disable SMT awareness in the scheduler.
    pub async fn set_smt_policy(self, enable: bool) -> Result<()> {
        self.context.xen.call.set_turbo_mode(CpuId::All, enable).await?;
        Ok(())
    }

    /// Set scheduler policy name.
    pub async fn set_scheduler_policy(self, policy: impl AsRef<str>) -> Result<()> {
        self.context.xen.call.set_cpufreq_gov(CpuId::All, policy).await?;
        Ok(())
    }
}
