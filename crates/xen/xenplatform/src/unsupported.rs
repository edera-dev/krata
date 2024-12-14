use xencall::sys::CreateDomain;

use crate::{
    boot::{BootDomain, BootSetupPlatform, DomainSegment},
    error::Result,
};

#[derive(Default, Clone)]
pub struct UnsupportedPlatform;

impl UnsupportedPlatform {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait::async_trait]
impl BootSetupPlatform for UnsupportedPlatform {
    fn create_domain(&self, _: bool) -> CreateDomain {
        panic!("unsupported platform")
    }

    fn page_size(&self) -> u64 {
        panic!("unsupported platform")
    }

    fn page_shift(&self) -> u64 {
        panic!("unsupported platform")
    }

    fn needs_early_kernel(&self) -> bool {
        panic!("unsupported platform")
    }

    fn hvm(&self) -> bool {
        panic!("unsupported platform")
    }

    async fn initialize_early(&mut self, _: &mut BootDomain) -> Result<()> {
        panic!("unsupported platform")
    }

    async fn initialize_memory(&mut self, _: &mut BootDomain) -> Result<()> {
        panic!("unsupported platform")
    }

    async fn alloc_page_tables(&mut self, _: &mut BootDomain) -> Result<Option<DomainSegment>> {
        panic!("unsupported platform")
    }

    async fn alloc_p2m_segment(&mut self, _: &mut BootDomain) -> Result<Option<DomainSegment>> {
        panic!("unsupported platform")
    }

    async fn alloc_magic_pages(&mut self, _: &mut BootDomain) -> Result<()> {
        panic!("unsupported platform")
    }

    async fn setup_page_tables(&mut self, _: &mut BootDomain) -> Result<()> {
        panic!("unsupported platform")
    }

    async fn setup_shared_info(&mut self, _: &mut BootDomain, _: u64) -> Result<()> {
        panic!("unsupported platform")
    }

    async fn setup_start_info(&mut self, _: &mut BootDomain, _: u64) -> Result<()> {
        panic!("unsupported platform")
    }

    async fn bootlate(&mut self, _: &mut BootDomain) -> Result<()> {
        panic!("unsupported platform")
    }

    async fn gnttab_seed(&mut self, _: &mut BootDomain) -> Result<()> {
        panic!("unsupported platform")
    }

    async fn vcpu(&mut self, _: &mut BootDomain) -> Result<()> {
        panic!("unsupported platform")
    }

    async fn setup_hypercall_page(&mut self, _: &mut BootDomain) -> Result<()> {
        panic!("unsupported platform")
    }
}
