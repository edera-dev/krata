use std::{net::Ipv6Addr, str::FromStr, time::Duration};

use anyhow::Result;
use hyphanet::icmp::{IcmpClient, IcmpProtocol};

#[tokio::main]
async fn main() -> Result<()> {
    let client = IcmpClient::new(IcmpProtocol::Icmpv6)?;
    let payload: [u8; 4] = [12u8, 14u8, 16u8, 32u8];
    let result = client
        .ping6(
            Ipv6Addr::from_str("2606:4700:4700::1111")?,
            0,
            1,
            &payload,
            Duration::from_secs(10),
        )
        .await?;
    println!("reply: {:?}", result);
    Ok(())
}
