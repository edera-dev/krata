use crate::error::{Error, Result};

pub fn vbd_blkidx_to_disk_name(blkid: u32) -> Result<String> {
    let mut name = "xvd".to_string();
    let mut suffix = String::new();
    let mut n = blkid;
    loop {
        let c = (n % 26) as u8;
        let c = b'a' + c;
        let c = char::from_u32(c as u32).ok_or(Error::InvalidBlockIdx)?;
        suffix.push(c);
        if n >= 26 {
            n = (n / 26) - 1;
            continue;
        } else {
            break;
        }
    }
    name.push_str(&suffix.chars().rev().collect::<String>());
    Ok(name)
}
