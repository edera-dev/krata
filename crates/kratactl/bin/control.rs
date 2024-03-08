use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand};
use env_logger::Env;
use krata::control::{
    guest_image_spec::Image, watch_events_reply::Event, DestroyGuestRequest, GuestImageSpec,
    GuestOciImageSpec, LaunchGuestRequest, ListGuestsRequest, WatchEventsRequest,
};
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
        oci: String,
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
    Watch {},
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("warn")).init();

    let args = ControllerArgs::parse();
    let mut client = ControlClientProvider::dial(args.connection.parse()?).await?;

    match args.command {
        Commands::Launch {
            oci,
            cpus,
            mem,
            attach,
            env,
            run,
        } => {
            let request = LaunchGuestRequest {
                image: Some(GuestImageSpec {
                    image: Some(Image::Oci(GuestOciImageSpec { image: oci })),
                }),
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
                let ipv4 = guest
                    .network
                    .as_ref()
                    .map(|x| x.ipv4.as_str())
                    .unwrap_or("unknown");
                let ipv6 = guest
                    .network
                    .as_ref()
                    .map(|x| x.ipv6.as_str())
                    .unwrap_or("unknown");
                let image = guest
                    .image
                    .map(|x| {
                        x.image
                            .map(|y| match y {
                                Image::Oci(oci) => oci.image,
                            })
                            .unwrap_or("unknown".to_string())
                    })
                    .unwrap_or("unknown".to_string());
                table.push_row_string(&vec![
                    guest.id,
                    ipv4.to_string(),
                    ipv6.to_string(),
                    image,
                ])?;
            }
            if table.num_records() == 1 {
                println!("no guests have been launched");
            } else {
                println!("{}", table.to_string());
            }
        }

        Commands::Watch {} => {
            let response = client
                .watch_events(Request::new(WatchEventsRequest {}))
                .await?;
            let mut stream = response.into_inner();
            while let Some(reply) = stream.message().await? {
                let Some(event) = reply.event else {
                    continue;
                };

                match event {
                    Event::GuestLaunched(launched) => {
                        println!("event=guest.launched guest={}", launched.guest_id);
                    }

                    Event::GuestDestroyed(destroyed) => {
                        println!("event=guest.destroyed guest={}", destroyed.guest_id);
                    }

                    Event::GuestExited(exited) => {
                        println!(
                            "event=guest.exited guest={} code={}",
                            exited.guest_id, exited.code
                        );
                    }
                }
            }
        }
    }
    Ok(())
}
