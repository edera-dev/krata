use clap::Parser;
use hypha::ctl::Controller;
use hypha::error::Result;
use hypha::image::ImageCompiler;
use ocipkg::ImageName;

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
    let mut controller = Controller::new(args.kernel, args.initrd, args.cpus, args.mem)?;
    let image = ImageName::parse(args.image.as_str())?;
    let compiler = ImageCompiler::new()?;
    let squashfs = compiler.compile(&image)?;
    println!("packed image into squashfs: {}", &squashfs);

    let domid = controller.launch()?;
    println!("launched domain: {}", domid);
    Ok(())
}
