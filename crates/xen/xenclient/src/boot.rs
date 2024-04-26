use crate::error::Result;
use crate::mem::PhysicalPages;
use crate::sys::{GrantEntry, XEN_PAGE_SHIFT};
use crate::Error;
use libc::munmap;
use log::debug;
use nix::errno::Errno;
use slice_copy::copy;

use crate::mem::ARCH_PAGE_SHIFT;
use std::ffi::c_void;
use std::slice;
use xencall::XenCall;

pub trait BootImageLoader {
    fn parse(&self) -> Result<BootImageInfo>;
    fn load(&self, image_info: &BootImageInfo, dst: &mut [u8]) -> Result<()>;
}

pub const XEN_UNSET_ADDR: u64 = -1i64 as u64;

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

pub struct BootSetup<'a> {
    pub(crate) call: &'a XenCall,
    pub phys: PhysicalPages<'a>,
    pub(crate) domid: u32,
    pub(crate) virt_alloc_end: u64,
    pub(crate) pfn_alloc_end: u64,
    pub(crate) virt_pgtab_end: u64,
    pub(crate) total_pages: u64,
    #[cfg(target_arch = "aarch64")]
    pub(crate) dtb: Option<Vec<u8>>,
}

#[derive(Debug)]
pub struct DomainSegment {
    pub(crate) vstart: u64,
    vend: u64,
    pub pfn: u64,
    pub(crate) addr: u64,
    pub(crate) size: u64,
    #[cfg(target_arch = "x86_64")]
    pub(crate) pages: u64,
}

#[derive(Debug)]
pub struct BootState {
    pub kernel_segment: DomainSegment,
    pub start_info_segment: DomainSegment,
    pub xenstore_segment: DomainSegment,
    pub boot_stack_segment: DomainSegment,
    pub p2m_segment: Option<DomainSegment>,
    pub page_table_segment: Option<DomainSegment>,
    pub image_info: BootImageInfo,
    pub shared_info_frame: u64,
    pub initrd_segment: DomainSegment,
    pub store_evtchn: u32,
    pub consoles: Vec<(u32, DomainSegment)>,
}

impl BootSetup<'_> {
    pub fn new(call: &XenCall, domid: u32) -> BootSetup {
        BootSetup {
            call,
            phys: PhysicalPages::new(call, domid),
            domid,
            virt_alloc_end: 0,
            pfn_alloc_end: 0,
            virt_pgtab_end: 0,
            total_pages: 0,
            #[cfg(target_arch = "aarch64")]
            dtb: None,
        }
    }

    async fn initialize_memory(
        &mut self,
        arch: &mut Box<dyn ArchBootSetup + Send + Sync>,
        total_pages: u64,
        kernel_segment: &Option<DomainSegment>,
        initrd_segment: &Option<DomainSegment>,
    ) -> Result<()> {
        arch.meminit(self, total_pages, kernel_segment, initrd_segment)
            .await?;
        Ok(())
    }

    async fn setup_hypercall_page(&mut self, image_info: &BootImageInfo) -> Result<()> {
        if image_info.virt_hypercall == XEN_UNSET_ADDR {
            return Ok(());
        }

        let pfn = (image_info.virt_hypercall - image_info.virt_base) >> ARCH_PAGE_SHIFT;
        let mfn = self.phys.p2m[pfn as usize];
        self.call.hypercall_init(self.domid, mfn).await?;
        Ok(())
    }

    pub async fn initialize<I: BootImageLoader + Send + Sync>(
        &mut self,
        arch: &mut Box<dyn ArchBootSetup + Send + Sync>,
        image_loader: &I,
        initrd: &[u8],
        max_vcpus: u32,
        mem_mb: u64,
        console_count: usize,
    ) -> Result<BootState> {
        debug!("initialize max_vcpus={:?} mem_mb={:?}", max_vcpus, mem_mb);

        let page_size = arch.page_size();
        let image_info = image_loader.parse()?;
        debug!("initialize image_info={:?}", image_info);
        let mut kernel_segment: Option<DomainSegment> = None;
        let mut initrd_segment: Option<DomainSegment> = None;
        if !image_info.unmapped_initrd {
            initrd_segment = Some(self.alloc_module(page_size, initrd).await?);
        }

        if arch.needs_early_kernel() {
            kernel_segment = Some(
                self.load_kernel_segment(page_size, image_loader, &image_info)
                    .await?,
            );
        }

        let total_pages = mem_mb << (20 - arch.page_shift());
        self.initialize_memory(arch, total_pages, &kernel_segment, &initrd_segment)
            .await?;
        self.virt_alloc_end = image_info.virt_base;

        if kernel_segment.is_none() {
            kernel_segment = Some(
                self.load_kernel_segment(page_size, image_loader, &image_info)
                    .await?,
            );
        }

        let mut p2m_segment: Option<DomainSegment> = None;
        if image_info.virt_p2m_base >= image_info.virt_base
            || (image_info.virt_p2m_base & ((1 << arch.page_shift()) - 1)) != 0
        {
            p2m_segment = arch.alloc_p2m_segment(self, &image_info).await?;
        }
        let start_info_segment = self.alloc_page(page_size)?;
        let xenstore_segment = self.alloc_page(page_size)?;
        let mut consoles: Vec<(u32, DomainSegment)> = Vec::new();
        for _ in 0..console_count {
            let evtchn = self.call.evtchn_alloc_unbound(self.domid, 0).await?;
            let page = self.alloc_page(page_size)?;
            consoles.push((evtchn, page));
        }
        let page_table_segment = arch.alloc_page_tables(self, &image_info).await?;
        let boot_stack_segment = self.alloc_page(page_size)?;

        if self.virt_pgtab_end > 0 {
            self.alloc_padding_pages(page_size, self.virt_pgtab_end)?;
        }

        if p2m_segment.is_none() {
            if let Some(mut segment) = arch.alloc_p2m_segment(self, &image_info).await? {
                segment.vstart = image_info.virt_p2m_base;
                p2m_segment = Some(segment);
            }
        }

        if image_info.unmapped_initrd {
            initrd_segment = Some(self.alloc_module(page_size, initrd).await?);
        }

        let initrd_segment = initrd_segment.unwrap();
        let store_evtchn = self.call.evtchn_alloc_unbound(self.domid, 0).await?;

        let kernel_segment =
            kernel_segment.ok_or(Error::MemorySetupFailed("kernel_segment missing"))?;

        let state = BootState {
            kernel_segment,
            start_info_segment,
            xenstore_segment,
            consoles,
            boot_stack_segment,
            p2m_segment,
            page_table_segment,
            image_info,
            initrd_segment,
            store_evtchn,
            shared_info_frame: 0,
        };
        debug!("initialize state={:?}", state);
        Ok(state)
    }

    pub async fn boot(
        &mut self,
        arch: &mut Box<dyn ArchBootSetup + Send + Sync>,
        state: &mut BootState,
        cmdline: &str,
    ) -> Result<()> {
        let domain_info = self.call.get_domain_info(self.domid).await?;
        let shared_info_frame = domain_info.shared_info_frame;
        state.shared_info_frame = shared_info_frame;
        arch.setup_page_tables(self, state).await?;
        arch.setup_start_info(self, state, cmdline).await?;
        self.setup_hypercall_page(&state.image_info).await?;
        arch.bootlate(self, state).await?;
        arch.setup_shared_info(self, state.shared_info_frame)
            .await?;
        arch.vcpu(self, state).await?;
        self.phys.unmap_all()?;
        self.gnttab_seed(state).await?;
        Ok(())
    }

    async fn gnttab_seed(&mut self, state: &mut BootState) -> Result<()> {
        let console_gfn =
            self.phys.p2m[state.consoles.first().map(|x| x.1.pfn).unwrap_or(0) as usize];
        let xenstore_gfn = self.phys.p2m[state.xenstore_segment.pfn as usize];
        let addr = self
            .call
            .mmap(0, 1 << XEN_PAGE_SHIFT)
            .await
            .ok_or(Error::MmapFailed)?;
        self.call.map_resource(self.domid, 1, 0, 0, 1, addr).await?;
        let entries = unsafe { slice::from_raw_parts_mut(addr as *mut GrantEntry, 2) };
        entries[0].flags = 1 << 0;
        entries[0].domid = 0;
        entries[0].frame = console_gfn as u32;
        entries[1].flags = 1 << 0;
        entries[1].domid = 0;
        entries[1].frame = xenstore_gfn as u32;
        unsafe {
            let result = munmap(addr as *mut c_void, 1 << XEN_PAGE_SHIFT);
            if result != 0 {
                return Err(Error::UnmapFailed(Errno::from_raw(result)));
            }
        }
        Ok(())
    }

    async fn load_kernel_segment<I: BootImageLoader + Send + Sync>(
        &mut self,
        page_size: u64,
        image_loader: &I,
        image_info: &BootImageInfo,
    ) -> Result<DomainSegment> {
        let kernel_segment = self
            .alloc_segment(
                page_size,
                image_info.virt_kstart,
                image_info.virt_kend - image_info.virt_kstart,
            )
            .await?;
        let kernel_segment_ptr = kernel_segment.addr as *mut u8;
        let kernel_segment_slice =
            unsafe { slice::from_raw_parts_mut(kernel_segment_ptr, kernel_segment.size as usize) };
        image_loader.load(image_info, kernel_segment_slice)?;
        Ok(kernel_segment)
    }

    pub(crate) fn round_up(addr: u64, mask: u64) -> u64 {
        addr | mask
    }

    #[cfg(target_arch = "x86_64")]
    pub(crate) fn bits_to_mask(bits: u64) -> u64 {
        (1 << bits) - 1
    }

    pub(crate) async fn alloc_segment(
        &mut self,
        page_size: u64,
        start: u64,
        size: u64,
    ) -> Result<DomainSegment> {
        debug!("alloc_segment {:#x} {:#x}", start, size);
        if start > 0 {
            self.alloc_padding_pages(page_size, start)?;
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

        self.chk_alloc_pages(page_size, pages)?;

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

    fn alloc_page(&mut self, page_size: u64) -> Result<DomainSegment> {
        let start = self.virt_alloc_end;
        let pfn = self.pfn_alloc_end;

        self.chk_alloc_pages(page_size, 1)?;
        debug!("alloc_page {:#x} (pfn {:#x})", start, pfn);
        Ok(DomainSegment {
            vstart: start,
            vend: (start + page_size) - 1,
            pfn,
            addr: 0,
            size: 0,
            #[cfg(target_arch = "x86_64")]
            pages: 1,
        })
    }

    async fn alloc_module(&mut self, page_size: u64, buffer: &[u8]) -> Result<DomainSegment> {
        let segment = self
            .alloc_segment(page_size, 0, buffer.len() as u64)
            .await?;
        let slice = unsafe { slice::from_raw_parts_mut(segment.addr as *mut u8, buffer.len()) };
        copy(slice, buffer);
        Ok(segment)
    }

    fn alloc_padding_pages(&mut self, page_size: u64, boundary: u64) -> Result<()> {
        if (boundary & (page_size - 1)) != 0 {
            return Err(Error::MemorySetupFailed("boundary is incorrect"));
        }

        if boundary < self.virt_alloc_end {
            return Err(Error::MemorySetupFailed("boundary is below allocation end"));
        }
        let pages = (boundary - self.virt_alloc_end) / page_size;
        self.chk_alloc_pages(page_size, pages)?;
        Ok(())
    }

    fn chk_alloc_pages(&mut self, page_size: u64, pages: u64) -> Result<()> {
        if pages > self.total_pages
            || self.pfn_alloc_end > self.total_pages
            || pages > self.total_pages - self.pfn_alloc_end
        {
            return Err(Error::MemorySetupFailed("no more pages left"));
        }

        self.pfn_alloc_end += pages;
        self.virt_alloc_end += pages * page_size;
        Ok(())
    }
}

#[async_trait::async_trait]
pub trait ArchBootSetup {
    fn page_size(&mut self) -> u64;
    fn page_shift(&mut self) -> u64;

    fn needs_early_kernel(&mut self) -> bool;

    async fn alloc_p2m_segment(
        &mut self,
        setup: &mut BootSetup,
        image_info: &BootImageInfo,
    ) -> Result<Option<DomainSegment>>;

    async fn alloc_page_tables(
        &mut self,
        setup: &mut BootSetup,
        image_info: &BootImageInfo,
    ) -> Result<Option<DomainSegment>>;

    async fn setup_page_tables(
        &mut self,
        setup: &mut BootSetup,
        state: &mut BootState,
    ) -> Result<()>;

    async fn setup_start_info(
        &mut self,
        setup: &mut BootSetup,
        state: &BootState,
        cmdline: &str,
    ) -> Result<()>;

    async fn setup_shared_info(
        &mut self,
        setup: &mut BootSetup,
        shared_info_frame: u64,
    ) -> Result<()>;

    async fn meminit(
        &mut self,
        setup: &mut BootSetup,
        total_pages: u64,
        kernel_segment: &Option<DomainSegment>,
        initrd_segment: &Option<DomainSegment>,
    ) -> Result<()>;
    async fn bootlate(&mut self, setup: &mut BootSetup, state: &mut BootState) -> Result<()>;
    async fn vcpu(&mut self, setup: &mut BootSetup, state: &mut BootState) -> Result<()>;
}
