use anyhow::Result;
use krata::v1::control::{ReadHypervisorConsoleReply, ReadHypervisorConsoleRequest};
use kratart::Runtime;

pub struct ReadHypervisorConsoleRpc {
    runtime: Runtime,
}

impl ReadHypervisorConsoleRpc {
    pub fn new(runtime: Runtime) -> Self {
        Self { runtime }
    }

    pub async fn process(
        self,
        _: ReadHypervisorConsoleRequest,
    ) -> Result<ReadHypervisorConsoleReply> {
        let data = self.runtime.read_hypervisor_console(false).await?;
        Ok(ReadHypervisorConsoleReply {
            data: data.to_string(),
        })
    }
}
