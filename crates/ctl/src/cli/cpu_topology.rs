use anyhow::Result;
use clap::Parser;
use krata::v1::control::{control_service_client::ControlServiceClient, HostCpuTopologyRequest};

use tonic::{transport::Channel, Request};

fn class_to_str(input: i32) -> String {
    match input {
        0 => "Standard".to_string(),
        1 => "Performance".to_string(),
        2 => "Efficiency".to_string(),
        _ => "???".to_string()
    }
}

#[derive(Parser)]
#[command(about = "Display information about a host's CPU topology")]
pub struct CpuTopologyCommand {}

impl CpuTopologyCommand {
    pub async fn run(self, mut client: ControlServiceClient<Channel>) -> Result<()> {
        println!("{0:<10} {1:<10} {2:<10} {3:<10} {4:<10} {5:<10}", "CPUID", "Node", "Socket", "Core", "Thread", "Class");

        let response = client
            .get_host_cpu_topology(Request::new(HostCpuTopologyRequest {}))
            .await?
            .into_inner();

        let mut i = 0;
        for cpu in response.cpus {
            println!("{0:<10} {1:<10} {2:<10} {3:<10} {4:<10} {5:<10}", i, cpu.node, cpu.socket, cpu.core, cpu.thread, class_to_str(cpu.class));
            i += 1;
        }
        
        Ok(())
    }
}
