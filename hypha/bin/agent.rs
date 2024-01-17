use hypha::agent::Agent;
use hypha::error::Result;

fn main() -> Result<()> {
    let mut agent = Agent::new()?;
    let domid = agent.launch()?;
    println!("launched domain: {}", domid);
    Ok(())
}
