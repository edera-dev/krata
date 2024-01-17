use clap::Parser;
use hypha::ctl::Controller;
use hypha::error::Result;

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
}

fn main() -> Result<()> {
    env_logger::init();

    let args = ControllerArgs::parse();
    let mut controller =
        Controller::new(args.kernel, args.initrd, args.image, args.cpus, args.mem)?;
    controller.compile()?;
    let domid = controller.launch()?;
    println!("launched domain: {}", domid);
    Ok(())
}
