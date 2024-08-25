use anyhow::Result;
use krata::v1::control::{SetHostPowerManagementPolicyReply, SetHostPowerManagementPolicyRequest};
use kratart::Runtime;

pub struct SetHostPowerManagementPolicyRpc {
    runtime: Runtime,
}

impl SetHostPowerManagementPolicyRpc {
    pub fn new(runtime: Runtime) -> Self {
        Self { runtime }
    }

    pub async fn process(
        self,
        request: SetHostPowerManagementPolicyRequest,
    ) -> Result<SetHostPowerManagementPolicyReply> {
        let power = self.runtime.power_management_context().await?;
        let scheduler = &request.scheduler;

        power.set_smt_policy(request.smt_awareness).await?;
        power.set_scheduler_policy(scheduler).await?;
        Ok(SetHostPowerManagementPolicyReply {})
    }
}
