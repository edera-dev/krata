use anyhow::Result;
use clap::{Parser, ValueEnum};
use krata::{
    events::EventStream,
    v1::control::{control_service_client::ControlServiceClient, SnoopIdmReply, SnoopIdmRequest},
};

use tokio_stream::StreamExt;
use tonic::transport::Channel;

use crate::format::{kv2line, proto2dynamic, proto2kv};

#[derive(ValueEnum, Clone, Debug, PartialEq, Eq)]
enum IdmSnoopFormat {
    Simple,
    Jsonl,
    KeyValue,
}

#[derive(Parser)]
#[command(about = "Snoop on the IDM bus")]
pub struct IdmSnoopCommand {
    #[arg(short, long, default_value = "simple", help = "Output format")]
    format: IdmSnoopFormat,
}

impl IdmSnoopCommand {
    pub async fn run(
        self,
        mut client: ControlServiceClient<Channel>,
        _events: EventStream,
    ) -> Result<()> {
        let mut stream = client.snoop_idm(SnoopIdmRequest {}).await?.into_inner();

        while let Some(reply) = stream.next().await {
            let reply = reply?;
            match self.format {
                IdmSnoopFormat::Simple => {
                    self.print_simple(reply)?;
                }

                IdmSnoopFormat::Jsonl => {
                    let value = serde_json::to_value(proto2dynamic(reply)?)?;
                    let encoded = serde_json::to_string(&value)?;
                    println!("{}", encoded.trim());
                }

                IdmSnoopFormat::KeyValue => {
                    self.print_key_value(reply)?;
                }
            }
        }

        Ok(())
    }

    fn print_simple(&self, reply: SnoopIdmReply) -> Result<()> {
        let from = reply.from;
        let to = reply.to;
        let Some(packet) = reply.packet else {
            return Ok(());
        };
        let value = serde_json::to_value(proto2dynamic(packet)?)?;
        let encoded = serde_json::to_string(&value)?;
        println!("({} -> {}) {}", from, to, encoded);
        Ok(())
    }

    fn print_key_value(&self, reply: SnoopIdmReply) -> Result<()> {
        let kvs = proto2kv(reply)?;
        println!("{}", kv2line(kvs));
        Ok(())
    }
}
