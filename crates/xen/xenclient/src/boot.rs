use std::slice;

use log::debug;
use slice_copy::copy;
use xencall::{sys::CreateDomain, XenCall};

use crate::{
    error::{Error, Result},
    mem::PhysicalPages,
    sys::XEN_PAGE_SHIFT,
};

pub struct BootSetup<I: BootImageLoader, P: BootSetupPlatform> {
    pub call: XenCall,
    pub domid: u32,
    pub platform: P,
    pub image_loader: I,
    pub dtb: Option<Vec<u8>>,
}

#[derive(Debug, Default, Clone)]
pub struct DomainSegment {
    pub vstart: u64,
    pub vend: u64,
    pub pfn: u64,
    pub addr: u64,
    pub size: u64,
    pub pages: u64,
}

pub struct BootDomain {
    pub domid: u32,
    pub call: XenCall,
    pub page_size: u64,
    pub virt_alloc_end: u64,
    pub pfn_alloc_end: u64,
    pub virt_pgtab_end: u64,
    pub total_pages: u64,
    pub target_pages: u64,
    pub max_vcpus: u32,
    pub image_info: BootImageInfo,
    pub phys: PhysicalPages,
    pub store_evtchn: u32,
    pub store_mfn: u64,
    pub initrd_segment: DomainSegment,
    pub consoles: Vec<(u32, u64)>,
    pub cmdline: String,
}

impl BootDomain {
    pub async fn alloc_module(&mut self, buffer: &[u8]) -> Result<DomainSegment> {
        let segment = self.alloc_segment(0, buffer.len() as u64).await?;
        let slice = unsafe { slice::from_raw_parts_mut(segment.addr as *mut u8, buffer.len()) };
        copy(slice, buffer);
        Ok(segment)
    }

    pub async fn alloc_segment(&mut self, start: u64, size: u64) -> Result<DomainSegment> {
        debug!("alloc_segment {:#x} {:#x}", start, size);
        if start > 0 {
            self.alloc_padding_pages(start)?;
        }

        let local_page_size: u32 = (1i64 << XEN_PAGE_SHIFT) as u32;
        let pages = (size + local_page_size as u64 - 1) / local_page_size as u64;
        let start = self.virt_alloc_end;

        let mut segment = DomainSegment {
            vstart: start,
            vend: 0,
            pfn: self.pfn_alloc_end,
            addr: 0,
            size,
            #[cfg(target_arch = "x86_64")]
            pages,
        };

        self.chk_alloc_pages(pages)?;

        let ptr = self.phys.pfn_to_ptr(segment.pfn, pages).await?;
        segment.addr = ptr;
        let slice = unsafe {
            slice::from_raw_parts_mut(ptr as *mut u8, (pages * local_page_size as u64) as usize)
        };
        slice.fill(0);
        segment.vend = self.virt_alloc_end;
        debug!(
            "alloc_segment {:#x} -> {:#x} (pfn {:#x} + {:#x} pages)",
            start, segment.vend, segment.pfn, pages
        );
        Ok(segment)
    }

    pub fn alloc_padding_pages(&mut self, boundary: u64) -> Result<()> {
        if (boundary & (self.page_size - 1)) != 0 {
            return Err(Error::MemorySetupFailed("boundary is incorrect"));
        }

        if boundary < self.virt_alloc_end {
            return Err(Error::MemorySetupFailed("boundary is below allocation end"));
        }
        let pages = (boundary - self.virt_alloc_end) / self.page_size;
        self.chk_alloc_pages(pages)?;
        Ok(())
    }

    pub fn chk_alloc_pages(&mut self, pages: u64) -> Result<()> {
        if pages > self.total_pages
            || self.pfn_alloc_end > self.total_pages
            || pages > self.total_pages - self.pfn_alloc_end
        {
            return Err(Error::MemorySetupFailed("no more pages left"));
        }

        self.pfn_alloc_end += pages;
        self.virt_alloc_end += pages * self.page_size;
        Ok(())
    }

    pub fn alloc_page(&mut self) -> Result<DomainSegment> {
        let start = self.virt_alloc_end;
        let pfn = self.pfn_alloc_end;

        self.chk_alloc_pages(1)?;
        debug!("alloc_page {:#x} (pfn {:#x})", start, pfn);
        Ok(DomainSegment {
            vstart: start,
            vend: (start + self.page_size) - 1,
            pfn,
            addr: 0,
            size: 0,
            pages: 1,
        })
    }

    pub fn round_up(addr: u64, mask: u64) -> u64 {
        addr | mask
    }

    pub fn bits_to_mask(bits: u64) -> u64 {
        (1 << bits) - 1
    }
}

impl<I: BootImageLoader, P: BootSetupPlatform> BootSetup<I, P> {
    pub fn new(
        call: XenCall,
        domid: u32,
        platform: P,
        image_loader: I,
        dtb: Option<Vec<u8>>,
    ) -> BootSetup<I, P> {
        BootSetup {
            call,
            domid,
            platform,
            image_loader,
            dtb,
        }
    }

    pub async fn initialize(
        &mut self,
        initrd: &[u8],
        mem_mb: u64,
        max_vcpus: u32,
        cmdline: &str,
    ) -> Result<BootDomain> {
        let total_pages = mem_mb << (20 - self.platform.page_shift());
        let image_info = self.image_loader.parse(true).await?;
        let mut domain = BootDomain {
            domid: self.domid,
            call: self.call.clone(),
            virt_alloc_end: 0,
            virt_pgtab_end: 0,
            pfn_alloc_end: 0,
            total_pages,
            target_pages: total_pages,
            page_size: self.platform.page_size(),
            image_info,
            consoles: Vec::new(),
            max_vcpus,
            phys: PhysicalPages::new(self.call.clone(), self.domid, self.platform.page_shift()),
            initrd_segment: DomainSegment::default(),
            store_evtchn: 0,
            store_mfn: 0,
            cmdline: cmdline.to_string(),
        };

        self.platform.initialize_early(&mut domain).await?;

        let mut initrd_segment = if !domain.image_info.unmapped_initrd {
            Some(domain.alloc_module(initrd).await?)
        } else {
            None
        };

        let mut kernel_segment = if self.platform.needs_early_kernel() {
            Some(self.load_kernel_segment(&mut domain).await?)
        } else {
            None
        };

        self.platform.initialize_memory(&mut domain).await?;
        domain.virt_alloc_end = domain.image_info.virt_base;

        if kernel_segment.is_none() {
            kernel_segment = Some(self.load_kernel_segment(&mut domain).await?);
        }

        if domain.image_info.unmapped_initrd {
            initrd_segment = Some(domain.alloc_module(initrd).await?);
        }

        domain.initrd_segment =
            initrd_segment.ok_or(Error::MemorySetupFailed("initrd_segment missing"))?;

        self.platform.alloc_magic_pages(&mut domain).await?;

        domain.store_evtchn = self.call.evtchn_alloc_unbound(self.domid, 0).await?;

        let _kernel_segment =
            kernel_segment.ok_or(Error::MemorySetupFailed("kernel_segment missing"))?;

        Ok(domain)
    }

    pub async fn boot(&mut self, domain: &mut BootDomain) -> Result<()> {
        let domain_info = self.call.get_domain_info(self.domid).await?;
        let shared_info_frame = domain_info.shared_info_frame;
        self.platform.setup_page_tables(domain).await?;
        self.platform
            .setup_start_info(domain, shared_info_frame)
            .await?;
        self.platform.setup_hypercall_page(domain).await?;
        self.platform.bootlate(domain).await?;
        self.platform
            .setup_shared_info(domain, shared_info_frame)
            .await?;
        self.platform.vcpu(domain).await?;
        domain.phys.unmap_all()?;
        self.platform.gnttab_seed(domain).await?;
        Ok(())
    }

    async fn load_kernel_segment(&mut self, domain: &mut BootDomain) -> Result<DomainSegment> {
        let kernel_segment = domain
            .alloc_segment(
                domain.image_info.virt_kstart,
                domain.image_info.virt_kend - domain.image_info.virt_kstart,
            )
            .await?;
        let kernel_segment_ptr = kernel_segment.addr as *mut u8;
        let kernel_segment_slice =
            unsafe { slice::from_raw_parts_mut(kernel_segment_ptr, kernel_segment.size as usize) };
        self.image_loader
            .load(&domain.image_info, kernel_segment_slice)
            .await?;
        Ok(kernel_segment)
    }
}

#[async_trait::async_trait]
pub trait BootSetupPlatform: Clone {
    fn create_domain(&self) -> CreateDomain;
    fn page_size(&self) -> u64;
    fn page_shift(&self) -> u64;
    fn needs_early_kernel(&self) -> bool;

    async fn initialize_early(&mut self, domain: &mut BootDomain) -> Result<()>;

    async fn initialize_memory(&mut self, domain: &mut BootDomain) -> Result<()>;

    async fn alloc_page_tables(&mut self, domain: &mut BootDomain)
        -> Result<Option<DomainSegment>>;

    async fn alloc_p2m_segment(&mut self, domain: &mut BootDomain)
        -> Result<Option<DomainSegment>>;

    async fn alloc_magic_pages(&mut self, domain: &mut BootDomain) -> Result<()>;

    async fn setup_page_tables(&mut self, domain: &mut BootDomain) -> Result<()>;

    async fn setup_shared_info(
        &mut self,
        domain: &mut BootDomain,
        shared_info_frame: u64,
    ) -> Result<()>;

    async fn setup_start_info(
        &mut self,
        domain: &mut BootDomain,
        shared_info_frame: u64,
    ) -> Result<()>;

    async fn bootlate(&mut self, domain: &mut BootDomain) -> Result<()>;

    async fn gnttab_seed(&mut self, domain: &mut BootDomain) -> Result<()>;

    async fn vcpu(&mut self, domain: &mut BootDomain) -> Result<()>;

    async fn setup_hypercall_page(&mut self, domain: &mut BootDomain) -> Result<()>;
}

#[async_trait::async_trait]
pub trait BootImageLoader {
    async fn parse(&self, hvm: bool) -> Result<BootImageInfo>;
    async fn load(&self, image_info: &BootImageInfo, dst: &mut [u8]) -> Result<()>;
}

#[derive(Debug)]
pub struct BootImageInfo {
    pub start: u64,
    pub virt_base: u64,
    pub virt_kstart: u64,
    pub virt_kend: u64,
    pub virt_hypercall: u64,
    pub virt_entry: u64,
    pub virt_p2m_base: u64,
    pub unmapped_initrd: bool,
}
