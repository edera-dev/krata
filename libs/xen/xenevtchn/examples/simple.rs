use xenevtchn::error::Result;
use xenevtchn::EventChannel;

fn main() -> Result<()> {
    let mut channel = EventChannel::open()?;
    println!("Channel opened.");
    let port = channel.bind_unbound_port(1)?;
    println!("port: {}", port);
    channel.unbind(port)?;
    Ok(())
}
