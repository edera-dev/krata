use regex::Regex;
use std::{fmt::Display, path::PathBuf, str::FromStr};
use tokio::fs;

use crate::error::{Error, Result};

const PCIBACK_SYSFS_PATH: &str = "/sys/bus/pci/drivers/pciback";
const PCI_BDF_REGEX: &str = r"^([0-9a-f]{4}):([0-9a-f]{2}):([0-9a-f]{2}).([0-9a-f]{1})$";
const PCI_BDF_SHORT_REGEX: &str = r"^([0-9a-f]{2}):([0-9a-f]{2}).([0-9a-f]{1})$";
const PCI_BDF_VDEFN_REGEX: &str =
    r"^([0-9a-f]{4}):([0-9a-f]{2}):([0-9a-f]{2}).([0-9a-f]{1})@([0-9a-f]{2})$";
const FLAG_PCI_BAR_IO: u64 = 0x1;

#[derive(Clone)]
pub struct XenPciBackend {
    path: PathBuf,
}

impl Default for XenPciBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl XenPciBackend {
    pub fn new() -> Self {
        Self {
            path: PathBuf::from(PCIBACK_SYSFS_PATH),
        }
    }

    pub async fn is_loaded(&self) -> Result<bool> {
        Ok(fs::try_exists(&self.path).await?)
    }

    pub async fn list_devices(&self) -> Result<Vec<PciBdf>> {
        let mut devices = Vec::new();
        let mut dir = fs::read_dir(&self.path).await?;
        while let Some(entry) = dir.next_entry().await? {
            let file_name_string = entry.file_name().to_string_lossy().to_string();
            let Some(bdf) = PciBdf::from_str(&file_name_string).ok() else {
                continue;
            };
            devices.push(bdf);
        }
        Ok(devices)
    }

    pub async fn is_assigned(&self, bdf: &PciBdf) -> Result<bool> {
        let mut path = self.path.clone();
        path.push(bdf.to_string());
        Ok(fs::try_exists(path).await?)
    }

    pub async fn read_resources(&self, bdf: &PciBdf) -> Result<Vec<PciMemoryResource>> {
        let mut resources = Vec::new();
        let mut path = self.path.clone();
        path.push(bdf.to_string());
        path.push("resource");
        let content = fs::read_to_string(&path).await?;
        for line in content.lines() {
            let parts = line.split(' ').collect::<Vec<_>>();
            if parts.len() != 3 {
                continue;
            }
            let Some(start) = parts.first() else {
                continue;
            };

            let Some(end) = parts.get(1) else {
                continue;
            };

            let Some(flags) = parts.get(2) else {
                continue;
            };

            if !start.starts_with("0x") || !end.starts_with("0x") || !flags.starts_with("0x") {
                continue;
            }

            let start = &start[2..];
            let end = &end[2..];
            let flags = &flags[2..];
            let Some(start) = u64::from_str_radix(start, 16).ok() else {
                continue;
            };
            let Some(end) = u64::from_str_radix(end, 16).ok() else {
                continue;
            };
            let Some(flags) = u64::from_str_radix(flags, 16).ok() else {
                continue;
            };

            if start > 0 {
                resources.push(PciMemoryResource::new(start, end, flags));
            }
        }
        Ok(resources)
    }

    pub async fn has_slot(&self, bdf: &PciBdf) -> Result<bool> {
        let mut slots_path = self.path.clone();
        slots_path.push("slots");
        let content = fs::read_to_string(&slots_path).await?;
        for line in content.lines() {
            if let Ok(slot) = PciBdf::from_str(line) {
                if slot == *bdf {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub struct PciBdf {
    pub domain: Option<u32>,
    pub bus: u16,
    pub device: u16,
    pub function: u16,
    pub vdefn: Option<u16>,
}

impl PciBdf {
    pub fn new(
        domain: Option<u32>,
        bus: u16,
        device: u16,
        function: u16,
        vdefn: Option<u16>,
    ) -> Self {
        Self {
            domain,
            bus,
            device,
            function,
            vdefn,
        }
    }

    pub fn with_domain(&self, domain: u32) -> PciBdf {
        PciBdf {
            domain: Some(domain),
            bus: self.bus,
            device: self.device,
            function: self.function,
            vdefn: self.vdefn,
        }
    }
}

impl FromStr for PciBdf {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        let pci_bdf_regex = Regex::from_str(PCI_BDF_REGEX)?;
        let pci_bdf_vdefn_regex = Regex::from_str(PCI_BDF_VDEFN_REGEX)?;
        let pci_bdf_short_regex = Regex::from_str(PCI_BDF_SHORT_REGEX)?;

        if let Some(pci_bdf_captures) = pci_bdf_regex.captures(s) {
            let domain = pci_bdf_captures
                .get(1)
                .ok_or_else(|| Error::GenericError("capture group 1 did not exist".to_string()))?;
            let bus = pci_bdf_captures
                .get(2)
                .ok_or_else(|| Error::GenericError("capture group 2 did not exist".to_string()))?;
            let device = pci_bdf_captures
                .get(3)
                .ok_or_else(|| Error::GenericError("capture group 3 did not exist".to_string()))?;
            let function = pci_bdf_captures
                .get(4)
                .ok_or_else(|| Error::GenericError("capture group 4 did not exist".to_string()))?;

            let domain = u32::from_str_radix(domain.as_str(), 16)?;
            let bus = u16::from_str_radix(bus.as_str(), 16)?;
            let device = u16::from_str_radix(device.as_str(), 16)?;
            let function = u16::from_str_radix(function.as_str(), 16)?;

            Ok(PciBdf::new(Some(domain), bus, device, function, None))
        } else if let Some(pci_bdf_vdefn_captures) = pci_bdf_vdefn_regex.captures(s) {
            let domain = pci_bdf_vdefn_captures
                .get(1)
                .ok_or_else(|| Error::GenericError("capture group 1 did not exist".to_string()))?;
            let bus = pci_bdf_vdefn_captures
                .get(2)
                .ok_or_else(|| Error::GenericError("capture group 2 did not exist".to_string()))?;
            let device = pci_bdf_vdefn_captures
                .get(3)
                .ok_or_else(|| Error::GenericError("capture group 3 did not exist".to_string()))?;
            let function = pci_bdf_vdefn_captures
                .get(4)
                .ok_or_else(|| Error::GenericError("capture group 4 did not exist".to_string()))?;
            let vdefn = pci_bdf_vdefn_captures
                .get(5)
                .ok_or_else(|| Error::GenericError("capture group 5 did not exist".to_string()))?;

            let domain = u32::from_str_radix(domain.as_str(), 16)?;
            let bus = u16::from_str_radix(bus.as_str(), 16)?;
            let device = u16::from_str_radix(device.as_str(), 16)?;
            let function = u16::from_str_radix(function.as_str(), 16)?;
            let vdefn = u16::from_str_radix(vdefn.as_str(), 16)?;
            Ok(PciBdf::new(
                Some(domain),
                bus,
                device,
                function,
                Some(vdefn),
            ))
        } else if let Some(pci_bdf_short_captures) = pci_bdf_short_regex.captures(s) {
            let bus = pci_bdf_short_captures
                .get(1)
                .ok_or_else(|| Error::GenericError("capture group 1 did not exist".to_string()))?;
            let device = pci_bdf_short_captures
                .get(2)
                .ok_or_else(|| Error::GenericError("capture group 2 did not exist".to_string()))?;
            let function = pci_bdf_short_captures
                .get(3)
                .ok_or_else(|| Error::GenericError("capture group 3 did not exist".to_string()))?;

            let bus = u16::from_str_radix(bus.as_str(), 16)?;
            let device = u16::from_str_radix(device.as_str(), 16)?;
            let function = u16::from_str_radix(function.as_str(), 16)?;
            Ok(PciBdf::new(None, bus, device, function, None))
        } else {
            Err(Error::InvalidPciBdfString)
        }
    }
}

impl Display for PciBdf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(domain) = self.domain {
            if let Some(vdefn) = self.vdefn {
                write!(
                    f,
                    "{:04x}:{:02x}:{:02x}.{:01x}@{:02x}",
                    domain, self.bus, self.device, self.function, vdefn
                )
            } else {
                write!(
                    f,
                    "{:04x}:{:02x}:{:02x}.{:01x}",
                    domain, self.bus, self.device, self.function
                )
            }
        } else {
            write!(
                f,
                "{:02x}:{:02x}.{:01x}",
                self.bus, self.device, self.function
            )
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PciMemoryResource {
    pub start: u64,
    pub end: u64,
    pub flags: u64,
}

impl PciMemoryResource {
    pub fn new(start: u64, end: u64, flags: u64) -> PciMemoryResource {
        PciMemoryResource { start, end, flags }
    }

    pub fn is_bar_io(&self) -> bool {
        (self.flags & FLAG_PCI_BAR_IO) != 0
    }

    pub fn size(&self) -> u64 {
        (self.end - self.start) + 1
    }
}
