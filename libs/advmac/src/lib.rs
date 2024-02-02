mod parser;

use arrayvec::ArrayString;
use core::fmt::{self, Debug, Display, Formatter};
use core::str::FromStr;
use rand::Rng;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

#[derive(Eq, PartialEq, Debug, Clone, Copy)]
pub enum ParseError {
    InvalidMac,
    InvalidLength { length: usize },
}

impl Display for ParseError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMac => write!(f, "invalid MAC address"),
            Self::InvalidLength { length } => write!(f, "invalid string length: {}", length),
        }
    }
}

impl std::error::Error for ParseError {}

#[derive(Eq, PartialEq, Debug, Clone, Copy)]
pub enum IpError {
    NotLinkLocal,
    NotMulticast,
}

impl Display for IpError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotLinkLocal => write!(f, "not link-local address"),
            Self::NotMulticast => write!(f, "not multicast address"),
        }
    }
}

impl std::error::Error for IpError {}

/// Maximum formatted size.
///
/// It is useful for creating a stack-allocated buffer `[u8; MAC_MAX_SIZE]`
/// and formatting address into it using [MacAddr6::format_write] or [MacAddr8::format_write].
pub const MAC_MAX_SIZE: usize = 23;
/// Size of formatted MAC using [MacAddr6::format_string] and [MacAddrFormat::Canonical].
pub const MAC_CANONICAL_SIZE6: usize = 17;
/// Size of formatted MAC using [MacAddr8::format_string] and [MacAddrFormat::Canonical].
pub const MAC_CANONICAL_SIZE8: usize = 23;
/// Size of formatted MAC using [MacAddr6::format_string] and [MacAddrFormat::ColonNotation].
pub const MAC_COLON_NOTATION_SIZE6: usize = 17;
/// Size of formatted MAC using [MacAddr8::format_string] and [MacAddrFormat::ColonNotation].
pub const MAC_COLON_NOTATION_SIZE8: usize = 23;
/// Size of formatted MAC using [MacAddr6::format_string] and [MacAddrFormat::DotNotation].
pub const MAC_DOT_NOTATION_SIZE6: usize = 14;
/// Size of formatted MAC using [MacAddr8::format_string] and [MacAddrFormat::DotNotation].
pub const MAC_DOT_NOTATION_SIZE8: usize = 19;
/// Size of formatted MAC using [MacAddr6::format_string] and [MacAddrFormat::Hexadecimal].
pub const MAC_HEXADECIMAL_SIZE6: usize = 12;
/// Size of formatted MAC using [MacAddr8::format_string] and [MacAddrFormat::Hexadecimal].
pub const MAC_HEXADECIMAL_SIZE8: usize = 16;
/// Size of formatted MAC using [MacAddr6::format_string] and [MacAddrFormat::Hexadecimal0x].
pub const MAC_HEXADECIMAL0X_SIZE6: usize = 14;
/// Size of formatted MAC using [MacAddr8::format_string] and [MacAddrFormat::Hexadecimal0x].
pub const MAC_HEXADECIMAL0X_SIZE8: usize = 18;

#[derive(Copy, Clone, Eq, PartialEq)]
pub enum MacAddrFormat {
    /// `AA-BB-CC-DD-EE-FF` (17 bytes) or `AA-BB-CC-DD-EE-FF-GG-HH` (23 bytes)
    Canonical,
    /// `AA:BB:CC:DD:EE:FF` (17 bytes) or `AA:BB:CC:DD:EE:FF:GG:HH` (23 bytes)
    ColonNotation,
    /// `AABB.CCDD.EEFF` (14 bytes) or `AABB.CCDD.EEFF.GGHH` (19 bytes)
    DotNotation,
    /// `AABBCCDDEEFF` (12 bytes) or `AABBCCDDEEFFGGHH` (16 bytes)
    Hexadecimal,
    /// `0xAABBCCDDEEFF` (14 bytes) or `0xAABBCCDDEEFFGGHH` (18 bytes)
    Hexadecimal0x,
}

macro_rules! mac_impl {
    ($nm:ident, $sz:literal, $hex_sz:literal) => {
        impl $nm {
            pub const fn new(eui: [u8; $sz]) -> Self {
                Self(eui)
            }

            pub fn random() -> Self {
                let mut result = Self::default();
                rand::rngs::OsRng.fill(result.as_mut_slice());
                result
            }

            pub const fn broadcast() -> Self {
                Self([0xFF; $sz])
            }

            pub const fn nil() -> Self {
                Self([0; $sz])
            }

            /// Sets *locally administered* flag
            pub fn set_local(&mut self, v: bool) {
                if v {
                    self.0[0] |= 0b0000_0010;
                } else {
                    self.0[0] &= !0b0000_0010;
                }
            }

            /// Returns the state of *locally administered* flag
            pub const fn is_local(&self) -> bool {
                (self.0[0] & 0b0000_0010) != 0
            }

            /// Sets *multicast* flag
            pub fn set_multicast(&mut self, v: bool) {
                if v {
                    self.0[0] |= 0b0000_0001;
                } else {
                    self.0[0] &= !0b0000_0001;
                }
            }

            /// Returns the state of *multicast* flag
            pub const fn is_multicast(&self) -> bool {
                (self.0[0] & 0b0000_0001) != 0
            }

            /// Returns [organizationally unique identifier (OUI)](https://en.wikipedia.org/wiki/Organizationally_unique_identifier) of this MAC address
            pub const fn oui(&self) -> [u8; 3] {
                [self.0[0], self.0[1], self.0[2]]
            }

            /// Sets [organizationally unique identifier (OUI)](https://en.wikipedia.org/wiki/Organizationally_unique_identifier) for this MAC address
            pub fn set_oui(&mut self, oui: [u8; 3]) {
                self.0[..3].copy_from_slice(&oui);
            }

            /// Returns internal array representation for this MAC address, consuming it
            pub const fn to_array(self) -> [u8; $sz] {
                self.0
            }

            /// Returns internal array representation for this MAC address as [u8] slice
            pub const fn as_slice(&self) -> &[u8] {
                &self.0
            }

            /// Returns internal array representation for this MAC address as mutable [u8] slice
            pub fn as_mut_slice(&mut self) -> &mut [u8] {
                &mut self.0
            }

            /// Returns internal array representation for this MAC address as [core::ffi::c_char] slice.
            /// This can be useful in parsing `ifr_hwaddr`, for example.
            pub const fn as_c_slice(&self) -> &[core::ffi::c_char] {
                unsafe { &*(self.as_slice() as *const _ as *const [core::ffi::c_char]) }
            }

            /// Parse MAC address from string and return it as `MacAddr`.
            /// This function can be used in const context, so MAC address can be parsed in compile-time.
            pub const fn parse_str(s: &str) -> Result<Self, ParseError> {
                match parser::MacParser::<$sz, $hex_sz>::parse(s) {
                    Ok(v) => Ok(Self(v)),
                    Err(e) => Err(e),
                }
            }

            /// Write MAC address to `impl core::fmt::Write`, which can be used in `no_std` environments.
            ///
            /// It can be used like this with [arrayvec::ArrayString] without allocations:
            /// ```
            /// use arrayvec::ArrayString;
            /// use advmac::{MacAddr6, MacAddrFormat, MAC_CANONICAL_SIZE6};
            ///
            /// let mac = MacAddr6::parse_str("AA:BB:CC:DD:EE:FF").unwrap();
            ///
            /// let mut buf = ArrayString::<MAC_CANONICAL_SIZE6>::new();
            /// mac.format_write(&mut buf, MacAddrFormat::Canonical).unwrap();
            /// # assert_eq!(buf.as_str(), "AA-BB-CC-DD-EE-FF")
            /// ```
            pub fn format_write<T: fmt::Write>(
                &self,
                f: &mut T,
                format: MacAddrFormat,
            ) -> fmt::Result {
                match format {
                    MacAddrFormat::Canonical => self.write_internal(f, "", "-", "-"),
                    MacAddrFormat::ColonNotation => self.write_internal(f, "", ":", ":"),
                    MacAddrFormat::DotNotation => self.write_internal(f, "", "", "."),
                    MacAddrFormat::Hexadecimal => self.write_internal(f, "", "", ""),
                    MacAddrFormat::Hexadecimal0x => self.write_internal(f, "0x", "", ""),
                }
            }

            /// Write MAC address to [String]. This function uses [Self::format_write] internally and
            /// produces the same result, but in string form, which can be convenient in non-constrainted
            /// environments.

            pub fn format_string(&self, format: MacAddrFormat) -> String {
                let mut buf = String::new();
                self.format_write(&mut buf, format).unwrap();
                buf
            }
        }

        impl Display for $nm {
            fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
                self.format_write(f, MacAddrFormat::Canonical)
            }
        }

        impl Debug for $nm {
            fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
                self.format_write(f, MacAddrFormat::Canonical)
            }
        }

        impl From<[u8; $sz]> for $nm {
            fn from(arr: [u8; $sz]) -> Self {
                Self(arr)
            }
        }

        impl TryFrom<&[u8]> for $nm {
            type Error = ParseError;

            fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
                Ok(Self(value.try_into().map_err(|_| ParseError::InvalidMac)?))
            }
        }

        impl TryFrom<&[core::ffi::c_char]> for $nm {
            type Error = ParseError;

            fn try_from(value: &[core::ffi::c_char]) -> Result<Self, Self::Error> {
                Self::try_from(unsafe { &*(value as *const _ as *const [u8]) })
            }
        }

        impl TryFrom<&str> for $nm {
            type Error = ParseError;

            fn try_from(value: &str) -> Result<Self, Self::Error> {
                Self::parse_str(value)
            }
        }

        impl TryFrom<String> for $nm {
            type Error = ParseError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::parse_str(&value)
            }
        }

        impl FromStr for $nm {
            type Err = ParseError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Self::parse_str(s)
            }
        }

        impl Serialize for $nm {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                let mut buf = ArrayString::<MAC_MAX_SIZE>::new();
                self.format_write(&mut buf, MacAddrFormat::Canonical)
                    .unwrap();
                s.serialize_str(buf.as_ref())
            }
        }

        impl<'de> Deserialize<'de> for $nm {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                Self::from_str(ArrayString::<MAC_MAX_SIZE>::deserialize(d)?.as_ref())
                    .map_err(serde::de::Error::custom)
            }
        }
    };
}

/// MAC address, represented as EUI-48
#[repr(transparent)]
#[derive(Default, Copy, Clone, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct MacAddr6([u8; 6]);
/// MAC address, represented as EUI-64
#[repr(transparent)]
#[derive(Default, Copy, Clone, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct MacAddr8([u8; 8]);

mac_impl!(MacAddr6, 6, 12);
mac_impl!(MacAddr8, 8, 16);

impl MacAddr6 {
    pub const fn to_modified_eui64(self) -> MacAddr8 {
        let b = self.to_array();
        MacAddr8([b[0] ^ 0b00000010, b[1], b[2], 0xFF, 0xFE, b[3], b[4], b[5]])
    }

    pub const fn try_from_modified_eui64(eui64: MacAddr8) -> Result<Self, IpError> {
        let b = eui64.to_array();
        if (b[3] == 0xFF) | (b[4] == 0xFE) {
            Ok(Self([b[0] ^ 0b00000010, b[1], b[2], b[5], b[6], b[7]]))
        } else {
            Err(IpError::NotLinkLocal)
        }
    }

    pub const fn to_link_local_ipv6(self) -> Ipv6Addr {
        let mac64 = self.to_modified_eui64().to_array();

        Ipv6Addr::new(
            0xFE80,
            0x0000,
            0x0000,
            0x0000,
            ((mac64[0] as u16) << 8) + mac64[1] as u16,
            ((mac64[2] as u16) << 8) + mac64[3] as u16,
            ((mac64[4] as u16) << 8) + mac64[5] as u16,
            ((mac64[6] as u16) << 8) + mac64[7] as u16,
        )
    }

    pub const fn try_from_link_local_ipv6(ip: Ipv6Addr) -> Result<Self, IpError> {
        let octets = ip.octets();
        if (octets[0] != 0xFE)
            | (octets[1] != 0x80)
            | (octets[2] != 0x00)
            | (octets[3] != 0x00)
            | (octets[4] != 0x00)
            | (octets[5] != 0x00)
            | (octets[6] != 0x00)
            | (octets[7] != 0x00)
            | (octets[11] != 0xFF)
            | (octets[12] != 0xFE)
        {
            return Err(IpError::NotLinkLocal);
        }

        Ok(Self([
            octets[8] ^ 0b00000010,
            octets[9],
            octets[10],
            octets[13],
            octets[14],
            octets[15],
        ]))
    }

    pub const fn try_from_multicast_ipv4(ip: Ipv4Addr) -> Result<Self, IpError> {
        if !ip.is_multicast() {
            return Err(IpError::NotMulticast);
        }
        let b = ip.octets();
        Ok(Self::new([0x01, 0x00, 0x5E, b[1] & 0x7F, b[2], b[3]]))
    }

    pub const fn try_from_multicast_ipv6(ip: Ipv6Addr) -> Result<Self, IpError> {
        if !ip.is_multicast() {
            return Err(IpError::NotMulticast);
        }
        let b = ip.octets();
        Ok(Self::new([0x33, 0x33, b[12], b[13], b[14], b[15]]))
    }

    pub const fn try_from_multicast_ip(ip: IpAddr) -> Result<Self, IpError> {
        match ip {
            IpAddr::V4(ip) => Self::try_from_multicast_ipv4(ip),
            IpAddr::V6(ip) => Self::try_from_multicast_ipv6(ip),
        }
    }
}

impl MacAddr6 {
    // String representations
    fn write_internal<T: fmt::Write>(
        &self,
        f: &mut T,
        pre: &str,
        sep: &str,
        sep2: &str,
    ) -> fmt::Result {
        write!(
            f,
            "{pre}{:02X}{sep}{:02X}{sep2}{:02X}{sep}{:02X}{sep2}{:02X}{sep}{:02X}",
            self.0[0], self.0[1], self.0[2], self.0[3], self.0[4], self.0[5]
        )
    }
}

impl MacAddr8 {
    // String representations
    fn write_internal<T: fmt::Write>(
        &self,
        f: &mut T,
        pre: &str,
        sep: &str,
        sep2: &str,
    ) -> fmt::Result {
        write!(
            f,
            "{pre}{:02X}{sep}{:02X}{sep2}{:02X}{sep}{:02X}{sep2}{:02X}{sep}{:02X}{sep2}{:02X}{sep}{:02X}",
            self.0[0], self.0[1], self.0[2], self.0[3], self.0[4], self.0[5], self.0[6], self.0[7]
        )
    }
}

/// Convenience macro for creating [MacAddr6] in compile-time.
///
/// Example:
/// ```
/// use advmac::{mac6, MacAddr6};
/// const MAC6: MacAddr6 = mac6!("11:22:33:44:55:66");
/// # assert_eq!(MAC6.to_array(), [0x11, 0x22, 0x33, 0x44, 0x55, 0x66]);
/// ```
#[macro_export]
macro_rules! mac6 {
    ($s:expr) => {
        match $crate::MacAddr6::parse_str($s) {
            Ok(mac) => mac,
            Err(_) => panic!("Invalid MAC address"),
        }
    };
}

/// Convenience macro for creating [MacAddr8] in compile-time.
///
/// Example:
/// ```
/// use advmac::{mac8, MacAddr8};
/// const MAC8: MacAddr8 = mac8!("11:22:33:44:55:66:77:88");
/// # assert_eq!(MAC8.to_array(), [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88]);
/// ```
#[macro_export]
macro_rules! mac8 {
    ($s:expr) => {
        match $crate::MacAddr8::parse_str($s) {
            Ok(mac) => mac,
            Err(_) => panic!("Invalid MAC address"),
        }
    };
}

#[cfg(test)]
mod test {
    #[test]
    fn test_flags_roundtrip() {
        let mut addr = mac6!("50:74:f2:b1:a8:7f");
        assert!(!addr.is_local());
        assert!(!addr.is_multicast());

        addr.set_multicast(true);
        assert!(!addr.is_local());
        assert!(addr.is_multicast());

        addr.set_local(true);
        assert!(addr.is_local());
        assert!(addr.is_multicast());

        addr.set_multicast(false);
        assert!(addr.is_local());
        assert!(!addr.is_multicast());

        addr.set_local(false);
        assert!(!addr.is_local());
        assert!(!addr.is_multicast());
    }
}
