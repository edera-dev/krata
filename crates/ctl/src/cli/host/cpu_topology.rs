use anyhow::Result;
use clap::{Parser, ValueEnum};
use comfy_table::presets::UTF8_FULL_CONDENSED;
use comfy_table::{Cell, Table};
use krata::v1::control::{
    control_service_client::ControlServiceClient, GetHostCpuTopologyRequest, HostCpuTopologyClass,
};

use tonic::{transport::Channel, Request};

fn class_to_str(input: HostCpuTopologyClass) -> String {
    match input {
        HostCpuTopologyClass::Standard => "Standard".to_string(),
        HostCpuTopologyClass::Performance => "Performance".to_string(),
        HostCpuTopologyClass::Efficiency => "Efficiency".to_string(),
    }
}

#[derive(ValueEnum, Clone, Debug, PartialEq, Eq)]
enum HostCpuTopologyFormat {
    Table,
}

#[derive(Parser)]
#[command(about = "Display information about the host CPU topology")]
pub struct HostCpuTopologyCommand {
    #[arg(short, long, default_value = "table", help = "Output format")]
    format: HostCpuTopologyFormat,
}

impl HostCpuTopologyCommand {
    pub async fn run(self, mut client: ControlServiceClient<Channel>) -> Result<()> {
        let response = client
            .get_host_cpu_topology(Request::new(GetHostCpuTopologyRequest {}))
            .await?
            .into_inner();

        let mut table = Table::new();
        table.load_preset(UTF8_FULL_CONDENSED);
        table.set_content_arrangement(comfy_table::ContentArrangement::Dynamic);
        table.set_header(vec!["id", "node", "socket", "core", "thread", "class"]);

        for (i, cpu) in response.cpus.iter().enumerate() {
            table.add_row(vec![
                Cell::new(i),
                Cell::new(cpu.node),
                Cell::new(cpu.socket),
                Cell::new(cpu.core),
                Cell::new(cpu.thread),
                Cell::new(class_to_str(cpu.class())),
            ]);
        }

        if !table.is_empty() {
            println!("{}", table);
        }

        Ok(())
    }
}
