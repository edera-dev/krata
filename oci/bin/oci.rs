use std::{path::PathBuf, str::FromStr};

use anyhow::{anyhow, Result};
use clap::Parser;
use krata::control::LaunchGuestRequest;
use krata::dial::ControlDialAddress;
use kratactl::client::ControlClientProvider;
use liboci_cli::StandardCmd;
use tokio::{
    fs::{self},
    process::Command,
};
use tonic::Request;

#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
    #[arg(long)]
    root: String,

    #[arg(long)]
    log: String,

    #[arg(long)]
    log_format: String,

    #[arg(long)]
    systemd_cgroup: bool,

    #[clap(subcommand)]
    subcommand: Subcommand,
}

#[derive(Parser, Debug)]
enum Subcommand {
    #[clap(flatten)]
    Standard(Box<liboci_cli::StandardCmd>),
    #[clap(flatten)]
    Common(Box<liboci_cli::CommonCmd>),
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    let address = ControlDialAddress::from_str("unix:///var/lib/krata/daemon.socket")?;
    let mut client = ControlClientProvider::dial(address).await?;

    let log_file = PathBuf::from(args.log);
    fs::write(&log_file, "").await?;

    match args.subcommand {
        Subcommand::Standard(cmd) => match *cmd {
            StandardCmd::Create(create) => {
                let pid_file = create.pid_file.unwrap();
                let response = client
                    .launch_guest(Request::new(LaunchGuestRequest {
                        image: "alpine:latest".to_string(),
                        mem: 512,
                        vcpus: 1,
                        env: vec![],
                        run: vec![],
                    }))
                    .await?
                    .into_inner();
                let Some(guest) = response.guest else {
                    return Err(anyhow!("krata failed to create a guest"));
                };
                let pid = spawn_stub_process(&guest.id).await?;
                fs::write(pid_file, pid.to_string()).await?;
            }

            _ => {}
        },

        _ => {}
    }

    Ok(())
}

async fn spawn_stub_process(guest: &str) -> Result<u32> {
    let mut stub = std::env::current_exe()?.parent().unwrap().to_path_buf();
    stub.push("krataoci-stub");
    let child = Command::new(stub).arg(guest).spawn()?;
    let rc = unsafe { libc::setsid() };
    if rc < 0 {
        return Err(anyhow!("failed to setsid: {}", rc));
    }
    Ok(child.id().unwrap())
}
