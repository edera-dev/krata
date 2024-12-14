use super::{DeviceConfig, DeviceDescription, DeviceResult, XenTransaction};
use crate::{
    error::{Error, Result},
    pci::{PciBdf, XenPciBackend},
};
use indexmap::IndexMap;
use xencall::{sys::DOMCTL_DEV_RDM_RELAXED, XenCall};
use xenplatform::sys::XEN_PAGE_SHIFT;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum PciRdmReservePolicy {
    Invalid,
    #[default]
    Strict,
    Relaxed,
}

impl PciRdmReservePolicy {
    pub fn to_option_str(&self) -> &str {
        match self {
            PciRdmReservePolicy::Invalid => "-1",
            PciRdmReservePolicy::Strict => "0",
            PciRdmReservePolicy::Relaxed => "1",
        }
    }
}

pub struct PciDeviceConfig {
    bdf: PciBdf,
    rdm_reserve_policy: PciRdmReservePolicy,
    permissive: bool,
    msi_translate: bool,
    power_management: bool,
}

pub struct PciRootDeviceConfig {
    backend_type: String,
    devices: Vec<PciDeviceConfig>,
}

impl PciDeviceConfig {
    pub fn new(bdf: PciBdf) -> Self {
        Self {
            bdf,
            rdm_reserve_policy: PciRdmReservePolicy::Strict,
            permissive: false,
            msi_translate: false,
            power_management: false,
        }
    }

    pub fn rdm_reserve_policy(&mut self, rdm_reserve_policy: PciRdmReservePolicy) -> &mut Self {
        self.rdm_reserve_policy = rdm_reserve_policy;
        self
    }

    pub fn permissive(&mut self, permissive: bool) -> &mut Self {
        self.permissive = permissive;
        self
    }

    pub fn msi_translate(&mut self, msi_translate: bool) -> &mut Self {
        self.msi_translate = msi_translate;
        self
    }

    pub fn power_management(&mut self, power_management: bool) -> &mut Self {
        self.power_management = power_management;
        self
    }

    pub fn done(self) -> Self {
        self
    }
}

impl Default for PciRootDeviceConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl PciRootDeviceConfig {
    pub fn new() -> Self {
        Self {
            backend_type: "pci".to_string(),
            devices: Vec::new(),
        }
    }

    pub fn backend_type(&mut self, backend_type: impl AsRef<str>) -> &mut Self {
        self.backend_type = backend_type.as_ref().to_string();
        self
    }

    pub fn add_device(&mut self, device: PciDeviceConfig) -> &mut Self {
        self.devices.push(device);
        self
    }

    pub async fn prepare(&self, domid: u32, call: &XenCall) -> Result<()> {
        for device in &self.devices {
            let backend = XenPciBackend::new();
            if !backend.is_assigned(&device.bdf).await? {
                return Err(Error::PciDeviceNotAssignable(device.bdf));
            }
            let resources = backend.read_resources(&device.bdf).await?;
            for resource in resources {
                if resource.is_bar_io() {
                    call.ioport_permission(
                        domid,
                        resource.start as u32,
                        resource.size() as u32,
                        true,
                    )
                    .await?;
                } else {
                    call.iomem_permission(
                        domid,
                        resource.start >> XEN_PAGE_SHIFT,
                        (resource.size() + (XEN_PAGE_SHIFT - 1)) >> XEN_PAGE_SHIFT,
                        true,
                    )
                    .await?;
                }
            }

            if let Some(irq) = backend.read_irq(&device.bdf).await? {
                let irq = call.map_pirq(domid, irq as isize, None).await?;
                call.irq_permission(domid, irq, true).await?;
            }

            backend.reset(&device.bdf).await?;

            call.assign_device(
                domid,
                device.bdf.encode(),
                if device.rdm_reserve_policy == PciRdmReservePolicy::Relaxed {
                    DOMCTL_DEV_RDM_RELAXED
                } else {
                    0
                },
            )
            .await?;

            if device.permissive {
                backend.enable_permissive(&device.bdf).await?;
            }
        }
        Ok(())
    }

    pub fn done(self) -> Self {
        self
    }
}

#[async_trait::async_trait]
impl DeviceConfig for PciRootDeviceConfig {
    type Result = DeviceResult;

    async fn add_to_transaction(&self, tx: &XenTransaction) -> Result<DeviceResult> {
        let id = tx.assign_next_devid().await?;
        let mut device = DeviceDescription::new("pci", &self.backend_type);
        device
            .add_backend_bool("online", true)
            .add_backend_item("state", 1)
            .add_backend_item("num_devs", self.devices.len());

        for (index, pci) in self.devices.iter().enumerate() {
            let mut options = IndexMap::new();
            options.insert("permissive", if pci.permissive { "1" } else { "0" });
            options.insert("rdm_policy", pci.rdm_reserve_policy.to_option_str());
            options.insert("msitranslate", if pci.msi_translate { "1" } else { "0" });
            let options = options
                .into_iter()
                .map(|(key, value)| format!("{}={}", key, value))
                .collect::<Vec<_>>()
                .join(",");
            device
                .add_backend_item(format!("key-{}", index), pci.bdf.to_string())
                .add_backend_item(format!("dev-{}", index), pci.bdf.to_string())
                .add_backend_item(format!("opts-{}", index), options);

            if let Some(vdefn) = pci.bdf.vdefn {
                device.add_backend_item(format!("vdefn-{}", index), format!("{:#x}", vdefn));
            }
        }

        device.add_frontend_item("state", 1);
        tx.add_device(id, device).await?;
        Ok(DeviceResult { id })
    }
}
