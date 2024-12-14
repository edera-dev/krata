use std::slice;

use log::debug;
use slice_copy::copy;
use xencall::{sys::CreateDomain, XenCall};

use crate::{
    error::{Error, Result},
    mem::PhysicalPages,
    sys::XEN_PAGE_SHIFT,
    ImageLoader, PlatformKernelConfig, PlatformResourcesConfig,
};

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
    pub initrd_segment: Option<DomainSegment>,
    pub console_evtchn: u32,
    pub console_mfn: u64,
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
        let pages = size.div_ceil(local_page_size as u64);
        let start = self.virt_alloc_end;

        let mut segment = DomainSegment {
            vstart: start,
            vend: 0,
            pfn: self.pfn_alloc_end,
            addr: 0,
            size,
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

#[async_trait::async_trait]
pub trait BootSetupPlatform {
    fn create_domain(&self, enable_iommu: bool) -> CreateDomain;
    fn page_size(&self) -> u64;
    fn page_shift(&self) -> u64;
    fn needs_early_kernel(&self) -> bool;
    fn hvm(&self) -> bool;

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

    async fn initialize_internal(
        &mut self,
        domid: u32,
        call: XenCall,
        image_loader: &ImageLoader,
        domain: &mut BootDomain,
        kernel: &PlatformKernelConfig,
    ) -> Result<()> {
        self.initialize_early(domain).await?;

        let mut initrd_segment = if !domain.image_info.unmapped_initrd && kernel.initrd.is_some() {
            Some(domain.alloc_module(kernel.initrd.as_ref().unwrap()).await?)
        } else {
            None
        };

        let mut kernel_segment = if self.needs_early_kernel() {
            Some(self.load_kernel_segment(image_loader, domain).await?)
        } else {
            None
        };

        self.initialize_memory(domain).await?;
        domain.virt_alloc_end = domain.image_info.virt_base;

        if kernel_segment.is_none() {
            kernel_segment = Some(self.load_kernel_segment(image_loader, domain).await?);
        }

        if domain.image_info.unmapped_initrd && kernel.initrd.is_some() {
            initrd_segment = Some(domain.alloc_module(kernel.initrd.as_ref().unwrap()).await?);
        }

        domain.initrd_segment = initrd_segment;
        self.alloc_magic_pages(domain).await?;
        domain.store_evtchn = call.evtchn_alloc_unbound(domid, 0).await?;
        let _kernel_segment =
            kernel_segment.ok_or(Error::MemorySetupFailed("kernel_segment missing"))?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn initialize(
        &mut self,
        domid: u32,
        call: XenCall,
        image_loader: &ImageLoader,
        kernel: &PlatformKernelConfig,
        resources: &PlatformResourcesConfig,
    ) -> Result<BootDomain> {
        let target_pages = resources.assigned_memory_mb << (20 - self.page_shift());
        let total_pages = resources.max_memory_mb << (20 - self.page_shift());
        let image_info = image_loader.parse(self.hvm()).await?;
        let mut domain = BootDomain {
            domid,
            call: call.clone(),
            virt_alloc_end: 0,
            virt_pgtab_end: 0,
            pfn_alloc_end: 0,
            total_pages,
            target_pages,
            page_size: self.page_size(),
            image_info,
            console_evtchn: 0,
            console_mfn: 0,
            max_vcpus: resources.max_vcpus,
            phys: PhysicalPages::new(call.clone(), domid, self.page_shift()),
            initrd_segment: None,
            store_evtchn: 0,
            store_mfn: 0,
            cmdline: kernel.cmdline.clone(),
        };
        match self
            .initialize_internal(domid, call, image_loader, &mut domain, kernel)
            .await
        {
            Ok(_) => Ok(domain),
            Err(error) => {
                domain.phys.unmap_all()?;
                Err(error)
            }
        }
    }

    async fn boot_internal(
        &mut self,
        call: XenCall,
        domid: u32,
        domain: &mut BootDomain,
    ) -> Result<()> {
        let domain_info = call.get_domain_info(domid).await?;
        let shared_info_frame = domain_info.shared_info_frame;
        self.setup_page_tables(domain).await?;
        self.setup_start_info(domain, shared_info_frame).await?;
        self.setup_hypercall_page(domain).await?;
        self.bootlate(domain).await?;
        self.setup_shared_info(domain, shared_info_frame).await?;
        self.vcpu(domain).await?;
        self.gnttab_seed(domain).await?;
        domain.phys.unmap_all()?;
        Ok(())
    }

    async fn boot(&mut self, domid: u32, call: XenCall, domain: &mut BootDomain) -> Result<()> {
        let result = self.boot_internal(call, domid, domain).await;
        domain.phys.unmap_all()?;
        result
    }

    async fn load_kernel_segment(
        &mut self,
        image_loader: &ImageLoader,
        domain: &mut BootDomain,
    ) -> Result<DomainSegment> {
        let kernel_segment = domain
            .alloc_segment(
                domain.image_info.virt_kstart,
                domain.image_info.virt_kend - domain.image_info.virt_kstart,
            )
            .await?;
        let kernel_segment_ptr = kernel_segment.addr as *mut u8;
        let kernel_segment_slice =
            unsafe { slice::from_raw_parts_mut(kernel_segment_ptr, kernel_segment.size as usize) };
        image_loader
            .load(&domain.image_info, kernel_segment_slice)
            .await?;
        Ok(kernel_segment)
    }
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
