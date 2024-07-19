use anyhow::Result;
use base64::Engine;
use clap::{Parser, ValueEnum};
use krata::{
    events::EventStream,
    idm::{internal, serialize::IdmSerializable, transport::IdmTransportPacketForm},
    v1::control::{control_service_client::ControlServiceClient, SnoopIdmReply, SnoopIdmRequest},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_stream::StreamExt;
use tonic::transport::Channel;

use crate::format::{kv2line, proto2dynamic, value2kv};

#[derive(ValueEnum, Clone, Debug, PartialEq, Eq)]
enum HostIdmSnoopFormat {
    Simple,
    Jsonl,
    KeyValue,
}

#[derive(Parser)]
#[command(about = "Snoop on the IDM bus")]
pub struct HostIdmSnoopCommand {
    #[arg(short, long, default_value = "simple", help = "Output format")]
    format: HostIdmSnoopFormat,
}

impl HostIdmSnoopCommand {
    pub async fn run(
        self,
        mut client: ControlServiceClient<Channel>,
        _events: EventStream,
    ) -> Result<()> {
        let mut stream = client.snoop_idm(SnoopIdmRequest {}).await?.into_inner();

        while let Some(reply) = stream.next().await {
            let reply = reply?;
            let Some(line) = convert_idm_snoop(reply) else {
                continue;
            };

            match self.format {
                HostIdmSnoopFormat::Simple => {
                    self.print_simple(line)?;
                }

                HostIdmSnoopFormat::Jsonl => {
                    let encoded = serde_json::to_string(&line)?;
                    println!("{}", encoded.trim());
                }

                HostIdmSnoopFormat::KeyValue => {
                    self.print_key_value(line)?;
                }
            }
        }

        Ok(())
    }

    fn print_simple(&self, line: IdmSnoopLine) -> Result<()> {
        let encoded = if !line.packet.decoded.is_null() {
            serde_json::to_string(&line.packet.decoded)?
        } else {
            base64::prelude::BASE64_STANDARD.encode(&line.packet.data)
        };
        println!(
            "({} -> {}) {} {} {}",
            line.from, line.to, line.packet.id, line.packet.form, encoded
        );
        Ok(())
    }

    fn print_key_value(&self, line: IdmSnoopLine) -> Result<()> {
        let kvs = value2kv(serde_json::to_value(line)?)?;
        println!("{}", kv2line(kvs));
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
pub struct IdmSnoopLine {
    pub from: String,
    pub to: String,
    pub packet: IdmSnoopData,
}

#[derive(Serialize, Deserialize)]
pub struct IdmSnoopData {
    pub id: u64,
    pub channel: u64,
    pub form: String,
    pub data: String,
    pub decoded: Value,
}

pub fn convert_idm_snoop(reply: SnoopIdmReply) -> Option<IdmSnoopLine> {
    let packet = &(reply.packet?);

    let decoded = if packet.channel == 0 {
        match packet.form() {
            IdmTransportPacketForm::Event => internal::Event::decode(&packet.data)
                .ok()
                .and_then(|event| proto2dynamic(event).ok()),

            IdmTransportPacketForm::Request
            | IdmTransportPacketForm::StreamRequest
            | IdmTransportPacketForm::StreamRequestUpdate => {
                internal::Request::decode(&packet.data)
                    .ok()
                    .and_then(|event| proto2dynamic(event).ok())
            }

            IdmTransportPacketForm::Response | IdmTransportPacketForm::StreamResponseUpdate => {
                internal::Response::decode(&packet.data)
                    .ok()
                    .and_then(|event| proto2dynamic(event).ok())
            }

            _ => None,
        }
    } else {
        None
    };

    let decoded = decoded
        .and_then(|message| serde_json::to_value(message).ok())
        .unwrap_or(Value::Null);

    let data = IdmSnoopData {
        id: packet.id,
        channel: packet.channel,
        form: match packet.form() {
            IdmTransportPacketForm::Raw => "raw".to_string(),
            IdmTransportPacketForm::Event => "event".to_string(),
            IdmTransportPacketForm::Request => "request".to_string(),
            IdmTransportPacketForm::Response => "response".to_string(),
            IdmTransportPacketForm::StreamRequest => "stream-request".to_string(),
            IdmTransportPacketForm::StreamRequestUpdate => "stream-request-update".to_string(),
            IdmTransportPacketForm::StreamRequestClosed => "stream-request-closed".to_string(),
            IdmTransportPacketForm::StreamResponseUpdate => "stream-response-update".to_string(),
            IdmTransportPacketForm::StreamResponseClosed => "stream-response-closed".to_string(),
            _ => format!("unknown-{}", packet.form),
        },
        data: base64::prelude::BASE64_STANDARD.encode(&packet.data),
        decoded,
    };

    Some(IdmSnoopLine {
        from: reply.from,
        to: reply.to,
        packet: data,
    })
}
