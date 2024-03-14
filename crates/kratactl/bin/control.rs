use anyhow::Result;
use clap::{Parser, Subcommand};
use env_logger::Env;
use krata::{
    common::{
        guest_image_spec::Image, GuestImageSpec, GuestOciImageSpec, GuestSpec, GuestState,
        GuestStatus,
    },
    control::{
        watch_events_reply::Event, CreateGuestRequest, DestroyGuestRequest, ListGuestsRequest,
        WatchEventsRequest,
    },
};
use kratactl::{client::ControlClientProvider, console::StdioConsoleStream, events::EventStream};
use log::error;
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
        #[arg(short, long)]
        name: Option<String>,
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
    let events = EventStream::open(
        client
            .watch_events(WatchEventsRequest {})
            .await?
            .into_inner(),
    )
    .await?;

    match args.command {
        Commands::Launch {
            name,
            oci,
            cpus,
            mem,
            attach,
            env,
            run,
        } => {
            let request = CreateGuestRequest {
                spec: Some(GuestSpec {
                    name: name.unwrap_or_default(),
                    image: Some(GuestImageSpec {
                        image: Some(Image::Oci(GuestOciImageSpec { image: oci })),
                    }),
                    vcpus: cpus,
                    mem,
                    env: env.unwrap_or_default(),
                    run,
                }),
            };
            let response = client
                .create_guest(Request::new(request))
                .await?
                .into_inner();
            let id = response.guest_id;
            if attach {
                wait_guest_started(&id, events.clone()).await?;
                let input = StdioConsoleStream::stdin_stream(id.clone()).await;
                let output = client.console_data(input).await?.into_inner();
                let exit_hook_task =
                    StdioConsoleStream::guest_exit_hook(id.clone(), events).await?;
                StdioConsoleStream::stdout(output).await?;
                exit_hook_task.abort();
            } else {
                println!("created guest: {}", id);
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
            let input = StdioConsoleStream::stdin_stream(guest.clone()).await;
            let output = client.console_data(input).await?.into_inner();
            let exit_hook_task = StdioConsoleStream::guest_exit_hook(guest.clone(), events).await?;
            StdioConsoleStream::stdout(output).await?;
            exit_hook_task.abort();
        }

        Commands::List { .. } => {
            let response = client
                .list_guests(Request::new(ListGuestsRequest {}))
                .await?
                .into_inner();
            let mut table = cli_tables::Table::new();
            let header = vec!["name", "uuid", "state", "ipv4", "ipv6", "image"];
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
                let Some(spec) = guest.spec else {
                    continue;
                };
                let image = spec
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
                    spec.name,
                    guest.id,
                    format!("{}", guest_state_text(guest.state.unwrap_or_default())),
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
            let mut stream = events.subscribe();
            loop {
                let event = stream.recv().await?;
                match event {
                    Event::GuestChanged(changed) => {
                        if let Some(guest) = changed.guest {
                            println!(
                                "event=guest.changed guest={} status={}",
                                guest.id,
                                guest_status_text(guest.state.unwrap_or_default().status())
                            );
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

fn guest_status_text(status: GuestStatus) -> String {
    match status {
        GuestStatus::Destroy => "destroying",
        GuestStatus::Destroyed => "destroyed",
        GuestStatus::Start => "starting",
        GuestStatus::Exited => "exited",
        GuestStatus::Started => "started",
        _ => "unknown",
    }
    .to_string()
}

fn guest_state_text(state: GuestState) -> String {
    let mut text = guest_status_text(state.status());

    if let Some(exit) = state.exit_info {
        text.push_str(&format!(" (exit code: {})", exit.code));
    }

    if let Some(error) = state.error_info {
        text.push_str(&format!(" (error: {})", error.message));
    }
    text
}

async fn wait_guest_started(id: &str, events: EventStream) -> Result<()> {
    let mut stream = events.subscribe();
    while let Ok(event) = stream.recv().await {
        match event {
            Event::GuestChanged(changed) => {
                let Some(guest) = changed.guest else {
                    continue;
                };

                if guest.id != id {
                    continue;
                }

                let Some(state) = guest.state else {
                    continue;
                };

                if let Some(ref error) = state.error_info {
                    error!("guest error: {}", error.message);
                }

                if state.status() == GuestStatus::Destroyed {
                    error!("guest destroyed");
                    std::process::exit(1);
                }

                if state.status() == GuestStatus::Started {
                    break;
                }
            }
        }
    }
    Ok(())
}
