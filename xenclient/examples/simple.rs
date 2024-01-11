use xenclient::create::{DomainConfig, PvDomainConfig};
use xenclient::{XenClient, XenClientError};

fn main() -> Result<(), XenClientError> {
    env_logger::init();

    let mut client = XenClient::open()?;
    let mut config = DomainConfig::new();
    config.configure_cpus(1);
    config.configure_memory(524288, 524288, 0);
    config.configure_pv(PvDomainConfig::new(
        "/boot/vmlinuz-6.1.0-17-amd64".to_string(),
        None,
        None,
    ));
    client.create(config)?;
    Ok(())
}
