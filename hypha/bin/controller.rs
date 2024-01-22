use clap::{Parser, Subcommand};
use hypha::ctl::Controller;
use hypha::error::{HyphaError, Result};
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
        #[arg(short, long)]
        image: String,
        #[arg(short, long, default_value_t = 1)]
        cpus: u32,
        #[arg(short, long, default_value_t = 512)]
        mem: u64,
    },
    Destroy {
        #[arg(short, long)]
        domain: u32,
    },
    Console {
        #[arg(short, long)]
        domain: u32,
    },
}

fn main() -> Result<()> {
    env_logger::init();

    let args = ControllerArgs::parse();
    let store_path = if args.store == "auto" {
        default_store_path()
            .ok_or_else(|| HyphaError::new("unable to determine default store path"))
    } else {
        Ok(PathBuf::from(args.store))
    }?;

    let store_path = store_path
        .to_str()
        .map(|x| x.to_string())
        .ok_or_else(|| HyphaError::new("unable to convert store path to string"))?;

    let mut controller = Controller::new(store_path.clone())?;

    match args.command {
        Commands::Launch {
            kernel,
            initrd,
            image,
            cpus,
            mem,
        } => {
            let kernel = map_kernel_path(&store_path, kernel);
            let initrd = map_initrd_path(&store_path, initrd);
            let domid = controller.launch(&kernel, &initrd, &image, cpus, mem)?;
            println!("launched domain: {}", domid);
        }

        Commands::Destroy { domain } => {
            controller.destroy(domain)?;
        }

        Commands::Console { domain } => {
            controller.console(domain)?;
        }

        Commands::List { .. } => {
            let containers = controller.list()?;
            let mut table = cli_tables::Table::new();
            let header = vec!["domain", "uuid", "image"];
            table.push_row(&header)?;
            for container in containers {
                let row = vec![
                    container.domid.to_string(),
                    container.uuid.to_string(),
                    container.image,
                ];
                table.push_row_string(&row)?;
            }
            println!("{}", table.to_string());
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
        path.push("/var/lib/hypha")
    } else {
        path.push(".hypha");
    }
    Some(path)
}
