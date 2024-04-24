use xenclient::pci::*;

use xenclient::error::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let backend = XenPciBackend::new();
    if !backend.is_loaded().await? {
        return Err(xenclient::error::Error::GenericError(
            "xen-pciback module not loaded".to_string(),
        ));
    }

    println!("assignable devices:");
    for device in backend.list_devices().await? {
        let is_assigned = backend.is_assigned(&device).await?;
        let has_slot = backend.has_slot(&device).await?;
        println!("{} slot={} assigned={}", device, has_slot, is_assigned);
        let resources = backend.read_resources(&device).await?;
        for resource in resources {
            println!(
                "  resource start={:#x} end={:#x} flags={:#x} bar-io={}",
                resource.start,
                resource.end,
                resource.flags,
                resource.is_bar_io()
            );
        }
    }

    Ok(())
}
