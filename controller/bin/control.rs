use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand};
use env_logger::Env;
use krata::control::{DestroyGuestRequest, LaunchGuestRequest, ListGuestsRequest};
use kratactl::{client::ControlClientProvider, console::StdioConsoleStream};
use tonic::Request;

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
    let mut client = ControlClientProvider::dial(args.connection.parse()?).await?;

    match args.command {
        Commands::Launch {
            image,
            cpus,
            mem,
            attach,
            env,
            run,
        } => {
            let request = LaunchGuestRequest {
                image,
                vcpus: cpus,
                mem,
                env: env.unwrap_or_default(),
                run,
            };
            let response = client
                .launch_guest(Request::new(request))
                .await?
                .into_inner();
            let Some(guest) = response.guest else {
                return Err(anyhow!(
                    "control service did not return a guest in the response"
                ));
            };
            println!("launched guest: {}", guest.id);
            if attach {
                let input = StdioConsoleStream::stdin_stream(guest.id).await;
                let output = client.console_data(input).await?.into_inner();
                StdioConsoleStream::stdout(output).await?;
            }
        }

        Commands::Destroy { guest } => {
            let _ = client
                .destroy_guest(Request::new(DestroyGuestRequest {
                    guest_id: guest.clone(),
                }))
                .await?
                .into_inner();
            println!("destroyed guest: {}", guest);
        }

        Commands::Console { guest } => {
            let input = StdioConsoleStream::stdin_stream(guest).await;
            let output = client.console_data(input).await?.into_inner();
            StdioConsoleStream::stdout(output).await?;
        }

        Commands::List { .. } => {
            let response = client
                .list_guests(Request::new(ListGuestsRequest {}))
                .await?
                .into_inner();
            let mut table = cli_tables::Table::new();
            let header = vec!["uuid", "ipv4", "ipv6", "image"];
            table.push_row(&header)?;
            for guest in response.guests {
                table.push_row_string(&vec![guest.id, guest.ipv4, guest.ipv6, guest.image])?;
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
