pub mod console;
pub mod destroy;
pub mod launch;
pub mod list;

impl From<crate::runtime::GuestInfo> for krata::control::GuestInfo {
    fn from(value: crate::runtime::GuestInfo) -> Self {
        krata::control::GuestInfo {
            id: value.uuid.to_string(),
            image: value.image.clone(),
            ipv4: value.ipv4.map(|x| x.ip().to_string()),
            ipv6: value.ipv6.map(|x| x.ip().to_string()),
        }
    }
}
