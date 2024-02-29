use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand};
use env_logger::Env;
use kratactrl::{
    ctl::{
        console::ControllerConsole, destroy::ControllerDestroy, launch::ControllerLaunch,
        ControllerContext,
    },
    launch::GuestLaunchRequest,
};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(version, about)]
struct ControllerArgs {
    #[arg(short, long, default_value = "auto")]
    store: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    List {},

    Launch {
        #[arg(short, long, default_value = "auto")]
        kernel: String,
        #[arg(short = 'r', long, default_value = "auto")]
        initrd: String,
        #[arg(short, long, default_value_t = 1)]
        cpus: u32,
        #[arg(short, long, default_value_t = 512)]
        mem: u64,
        #[arg[short, long]]
        env: Option<Vec<String>>,
        #[arg(short, long)]
        attach: bool,
        #[arg(long)]
        debug: bool,
        #[arg()]
        image: String,
        #[arg(allow_hyphen_values = true, trailing_var_arg = true)]
        run: Vec<String>,
    },
    Destroy {
        #[arg()]
        container: String,
    },
    Console {
        #[arg()]
        container: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("warn")).init();

    let args = ControllerArgs::parse();
    let store_path = if args.store == "auto" {
        default_store_path().ok_or_else(|| anyhow!("unable to determine default store path"))
    } else {
        Ok(PathBuf::from(args.store))
    }?;

    let store_path = store_path
        .to_str()
        .map(|x| x.to_string())
        .ok_or_else(|| anyhow!("unable to convert store path to string"))?;

    let mut context = ControllerContext::new(store_path.clone()).await?;

    match args.command {
        Commands::Launch {
            kernel,
            initrd,
            image,
            cpus,
            mem,
            attach,
            env,
            run,
            debug,
        } => {
            let kernel = map_kernel_path(&store_path, kernel);
            let initrd = map_initrd_path(&store_path, initrd);
            let mut launch = ControllerLaunch::new(&mut context);
            let request = GuestLaunchRequest {
                kernel_path: &kernel,
                initrd_path: &initrd,
                image: &image,
                vcpus: cpus,
                mem,
                env,
                run: if run.is_empty() { None } else { Some(run) },
                debug,
            };
            let info = launch.perform(request).await?;
            println!("launched guest: {}", info.uuid);
            if attach {
                let mut console = ControllerConsole::new(&mut context);
                console.perform(&info.uuid.to_string()).await?;
            }
        }

        Commands::Destroy { container } => {
            let mut destroy = ControllerDestroy::new(&mut context);
            destroy.perform(&container).await?;
        }

        Commands::Console { container } => {
            let mut console = ControllerConsole::new(&mut context);
            console.perform(&container).await?;
        }

        Commands::List { .. } => {
            let containers = context.list().await?;
            let mut table = cli_tables::Table::new();
            let header = vec!["uuid", "ipv4", "ipv6", "image"];
            table.push_row(&header)?;
            for container in containers {
                let row = vec![
                    container.uuid.to_string(),
                    container.ipv4,
                    container.ipv6,
                    container.image,
                ];
                table.push_row_string(&row)?;
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

fn map_kernel_path(store: &str, value: String) -> String {
    if value == "auto" {
        return format!("{}/default/kernel", store);
    }
    value
}

fn map_initrd_path(store: &str, value: String) -> String {
    if value == "auto" {
        return format!("{}/default/initrd", store);
    }
    value
}

fn default_store_path() -> Option<PathBuf> {
    let user_dirs = directories::UserDirs::new()?;
    let mut path = user_dirs.home_dir().to_path_buf();
    if path == PathBuf::from("/root") {
        path.push("/var/lib/krata")
    } else {
        path.push(".krata");
    }
    Some(path)
}
