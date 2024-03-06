use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand};
use env_logger::Env;
use krata::control::{
    ConsoleStreamRequest, DestroyRequest, LaunchRequest, ListRequest, Request, Response,
};
use kratactl::{
    client::{KrataClient, KrataClientTransport},
    console::XenConsole,
};
use url::Url;

#[derive(Parser, Debug)]
#[command(version, about)]
struct ControllerArgs {
    #[arg(short, long, default_value = "unix:///var/lib/krata/daemon.socket")]
    connection: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    List {},

    Launch {
        #[arg(short, long, default_value_t = 1)]
        cpus: u32,
        #[arg(short, long, default_value_t = 512)]
        mem: u64,
        #[arg[short, long]]
        env: Option<Vec<String>>,
        #[arg(short, long)]
        attach: bool,
        #[arg()]
        image: String,
        #[arg(allow_hyphen_values = true, trailing_var_arg = true)]
        run: Vec<String>,
    },
    Destroy {
        #[arg()]
        guest: String,
    },
    Console {
        #[arg()]
        guest: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("warn")).init();

    let args = ControllerArgs::parse();
    let transport = KrataClientTransport::dial(Url::parse(&args.connection)?).await?;
    let client = KrataClient::new(transport).await?;

    match args.command {
        Commands::Launch {
            image,
            cpus,
            mem,
            attach,
            env,
            run,
        } => {
            let request = LaunchRequest {
                image,
                vcpus: cpus,
                mem,
                env,
                run: if run.is_empty() { None } else { Some(run) },
            };
            let Response::Launch(response) = client.send(Request::Launch(request)).await? else {
                return Err(anyhow!("invalid response type"));
            };
            println!("launched guest: {}", response.guest.id);
            if attach {
                let request = ConsoleStreamRequest {
                    guest: response.guest.id.clone(),
                };
                let Response::ConsoleStream(response) =
                    client.send(Request::ConsoleStream(request)).await?
                else {
                    return Err(anyhow!("invalid response type"));
                };
                let stream = client.acquire(response.stream).await?;
                let console = XenConsole::new(stream).await?;
                console.attach().await?;
            }
        }

        Commands::Destroy { guest } => {
            let request = DestroyRequest { guest };
            let Response::Destroy(response) = client.send(Request::Destroy(request)).await? else {
                return Err(anyhow!("invalid response type"));
            };
            println!("destroyed guest: {}", response.guest);
        }

        Commands::Console { guest } => {
            let request = ConsoleStreamRequest { guest };
            let Response::ConsoleStream(response) =
                client.send(Request::ConsoleStream(request)).await?
            else {
                return Err(anyhow!("invalid response type"));
            };
            let stream = client.acquire(response.stream).await?;
            let console = XenConsole::new(stream).await?;
            console.attach().await?;
        }

        Commands::List { .. } => {
            let request = ListRequest {};
            let Response::List(response) = client.send(Request::List(request)).await? else {
                return Err(anyhow!("invalid response type"));
            };
            let mut table = cli_tables::Table::new();
            let header = vec!["uuid", "ipv4", "ipv6", "image"];
            table.push_row(&header)?;
            for guest in response.guests {
                table.push_row_string(&vec![
                    guest.id,
                    guest.ipv4.unwrap_or("none".to_string()),
                    guest.ipv6.unwrap_or("none".to_string()),
                    guest.image,
                ])?;
            }
            if table.num_records() == 1 {
                println!("no guests have been launched");
            } else {
                println!("{}", table.to_string());
            }
        }
    }
    Ok(())
}
