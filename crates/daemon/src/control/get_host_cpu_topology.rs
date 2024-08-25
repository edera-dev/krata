use anyhow::Result;
use krata::v1::control::{GetHostCpuTopologyReply, GetHostCpuTopologyRequest, HostCpuTopologyInfo};
use kratart::Runtime;

pub struct GetHostCpuTopologyRpc {
    runtime: Runtime,
}

impl GetHostCpuTopologyRpc {
    pub fn new(runtime: Runtime) -> Self {
        Self { runtime }
    }

    pub async fn process(
        self,
        _request: GetHostCpuTopologyRequest,
    ) -> Result<GetHostCpuTopologyReply> {
        let power = self.runtime.power_management_context().await?;
        let cpu_topology = power.cpu_topology().await?;
        let mut cpus = vec![];

        for cpu in cpu_topology {
            cpus.push(HostCpuTopologyInfo {
                core: cpu.core,
                socket: cpu.socket,
                node: cpu.node,
                thread: cpu.thread,
                class: cpu.class as i32,
            })
        }
        Ok(GetHostCpuTopologyReply { cpus })
    }
}
