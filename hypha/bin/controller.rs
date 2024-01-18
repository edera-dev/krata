use clap::Parser;
use hypha::ctl::Controller;
use hypha::error::{HyphaError, Result};

#[derive(Parser, Debug)]
#[command(version, about)]
struct ControllerArgs {
    #[arg(short, long)]
    kernel: String,

    #[arg(short = 'r', long)]
    initrd: String,

    #[arg(short, long)]
    image: String,

    #[arg(short, long, default_value_t = 1)]
    cpus: u32,

    #[arg(short, long, default_value_t = 512)]
    mem: u64,

    #[arg(short = 'C', long, default_value = "auto")]
    cache: String,
}

fn main() -> Result<()> {
    env_logger::init();

    let args = ControllerArgs::parse();
    let cache_path = if args.cache == "auto" {
        default_cache_path()
            .ok_or_else(|| HyphaError::new("unable to determine default cache path"))
    } else {
        Ok(args.cache)
    }?;

    let mut controller = Controller::new(
        cache_path,
        args.kernel,
        args.initrd,
        args.image,
        args.cpus,
        args.mem,
    )?;
    let domid = controller.launch()?;
    println!("launched domain: {}", domid);
    Ok(())
}

fn default_cache_path() -> Option<String> {
    let user_dirs = directories::UserDirs::new()?;
    let mut path = user_dirs.home_dir().to_path_buf();
    path.push(".hypha/cache");
    Some(path.to_str()?.to_string())
}
