use clap::Parser;
use hypha::agent::Agent;
use hypha::error::Result;

#[derive(Parser, Debug)]
#[command(version, about)]
struct AgentArgs {
    #[arg(short, long)]
    kernel: String,

    #[arg(short, long)]
    initrd: String,

    #[arg(short, long, default_value_t = 1)]
    cpus: u32,

    #[arg(short, long, default_value_t = 512)]
    mem: u64,
}

fn main() -> Result<()> {
    let args = AgentArgs::parse();
    let mut agent = Agent::new(args.kernel, args.initrd, args.cpus, args.mem)?;
    let domid = agent.launch()?;
    println!("launched domain: {}", domid);
    Ok(())
}
