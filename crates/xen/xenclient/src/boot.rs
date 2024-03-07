use crate::error::Result;
use crate::mem::PhysicalPages;
use crate::sys::{GrantEntry, XEN_PAGE_SHIFT};
use crate::Error;
use libc::munmap;
use log::debug;
use slice_copy::copy;

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
}

#[derive(Debug)]
pub struct DomainSegment {
    pub(crate) vstart: u64,
    vend: u64,
    pub pfn: u64,
    pub(crate) addr: u64,
    pub(crate) size: u64,
    pub(crate) pages: u64,
}

#[derive(Debug)]
pub struct BootState {
    pub kernel_segment: DomainSegment,
    pub start_info_segment: DomainSegment,
    pub xenstore_segment: DomainSegment,
    pub console_segment: DomainSegment,
    pub boot_stack_segment: DomainSegment,
    pub p2m_segment: DomainSegment,
    pub page_table_segment: DomainSegment,
    pub image_info: BootImageInfo,
    pub shared_info_frame: u64,
    pub initrd_segment: DomainSegment,
    pub store_evtchn: u32,
    pub console_evtchn: u32,
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
        }
    }

    fn initialize_memory(&mut self, arch: &mut dyn ArchBootSetup, total_pages: u64) -> Result<()> {
        self.call.set_address_size(self.domid, 64)?;
        arch.meminit(self, total_pages)?;
        Ok(())
    }

    pub fn initialize(
        &mut self,
        arch: &mut dyn ArchBootSetup,
        image_loader: &dyn BootImageLoader,
        initrd: &[u8],
        max_vcpus: u32,
        mem_mb: u64,
    ) -> Result<BootState> {
        debug!("initialize max_vcpus={:?} mem_mb={:?}", max_vcpus, mem_mb);

        let total_pages = mem_mb << (20 - arch.page_shift());
        self.initialize_memory(arch, total_pages)?;

        let image_info = image_loader.parse()?;
        debug!("initialize image_info={:?}", image_info);
        self.virt_alloc_end = image_info.virt_base;
        let kernel_segment = self.load_kernel_segment(arch, image_loader, &image_info)?;
        let mut p2m_segment: Option<DomainSegment> = None;
        if image_info.virt_p2m_base >= image_info.virt_base
            || (image_info.virt_p2m_base & ((1 << arch.page_shift()) - 1)) != 0
        {
            p2m_segment = Some(arch.alloc_p2m_segment(self, &image_info)?);
        }
        let start_info_segment = self.alloc_page(arch)?;
        let xenstore_segment = self.alloc_page(arch)?;
        let console_segment = self.alloc_page(arch)?;
        let page_table_segment = arch.alloc_page_tables(self, &image_info)?;
        let boot_stack_segment = self.alloc_page(arch)?;

        if self.virt_pgtab_end > 0 {
            self.alloc_padding_pages(arch, self.virt_pgtab_end)?;
        }

        let mut initrd_segment: Option<DomainSegment> = None;
        if !image_info.unmapped_initrd {
            initrd_segment = Some(self.alloc_module(arch, initrd)?);
        }
        if p2m_segment.is_none() {
            let mut segment = arch.alloc_p2m_segment(self, &image_info)?;
            segment.vstart = image_info.virt_p2m_base;
            p2m_segment = Some(segment);
        }
        let p2m_segment = p2m_segment.unwrap();

        if image_info.unmapped_initrd {
            initrd_segment = Some(self.alloc_module(arch, initrd)?);
        }

        let initrd_segment = initrd_segment.unwrap();
        let store_evtchn = self.call.evtchn_alloc_unbound(self.domid, 0)?;
        let console_evtchn = self.call.evtchn_alloc_unbound(self.domid, 0)?;
        let state = BootState {
            kernel_segment,
            start_info_segment,
            xenstore_segment,
            console_segment,
            boot_stack_segment,
            p2m_segment,
            page_table_segment,
            image_info,
            initrd_segment,
            store_evtchn,
            console_evtchn,
            shared_info_frame: 0,
        };
        debug!("initialize state={:?}", state);
        Ok(state)
    }

    pub fn boot(
        &mut self,
        arch: &mut dyn ArchBootSetup,
        state: &mut BootState,
        cmdline: &str,
    ) -> Result<()> {
        let domain_info = self.call.get_domain_info(self.domid)?;
        let shared_info_frame = domain_info.shared_info_frame;
        state.shared_info_frame = shared_info_frame;
        arch.setup_page_tables(self, state)?;
        arch.setup_start_info(self, state, cmdline)?;
        arch.setup_hypercall_page(self, &state.image_info)?;
        arch.bootlate(self, state)?;
        arch.setup_shared_info(self, state.shared_info_frame)?;
        arch.vcpu(self, state)?;
        self.phys.unmap_all()?;
        self.gnttab_seed(state)?;
        Ok(())
    }

    fn gnttab_seed(&mut self, state: &mut BootState) -> Result<()> {
        let console_gfn = self.phys.p2m[state.console_segment.pfn as usize];
        let xenstore_gfn = self.phys.p2m[state.xenstore_segment.pfn as usize];
        let addr = self
            .call
            .mmap(0, 1 << XEN_PAGE_SHIFT)
            .ok_or(Error::MmapFailed)?;
        self.call.map_resource(self.domid, 1, 0, 0, 1, addr)?;
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
                return Err(Error::UnmapFailed);
            }
        }
        Ok(())
    }

    fn load_kernel_segment(
        &mut self,
        arch: &mut dyn ArchBootSetup,
        image_loader: &dyn BootImageLoader,
        image_info: &BootImageInfo,
    ) -> Result<DomainSegment> {
        let kernel_segment = self.alloc_segment(
            arch,
            image_info.virt_kstart,
            image_info.virt_kend - image_info.virt_kstart,
        )?;
        let kernel_segment_ptr = kernel_segment.addr as *mut u8;
        let kernel_segment_slice =
            unsafe { slice::from_raw_parts_mut(kernel_segment_ptr, kernel_segment.size as usize) };
        image_loader.load(image_info, kernel_segment_slice)?;
        Ok(kernel_segment)
    }

    pub(crate) fn round_up(addr: u64, mask: u64) -> u64 {
        addr | mask
    }

    pub(crate) fn bits_to_mask(bits: u64) -> u64 {
        (1 << bits) - 1
    }

    pub(crate) fn alloc_segment(
        &mut self,
        arch: &mut dyn ArchBootSetup,
        start: u64,
        size: u64,
    ) -> Result<DomainSegment> {
        if start > 0 {
            self.alloc_padding_pages(arch, start)?;
        }

        let page_size: u32 = (1i64 << XEN_PAGE_SHIFT) as u32;
        let pages = (size + page_size as u64 - 1) / page_size as u64;
        let start = self.virt_alloc_end;

        let mut segment = DomainSegment {
            vstart: start,
            vend: 0,
            pfn: self.pfn_alloc_end,
            addr: 0,
            size,
            pages,
        };

        self.chk_alloc_pages(arch, pages)?;

        let ptr = self.phys.pfn_to_ptr(segment.pfn, pages)?;
        segment.addr = ptr;
        let slice = unsafe {
            slice::from_raw_parts_mut(ptr as *mut u8, (pages * page_size as u64) as usize)
        };
        slice.fill(0);
        segment.vend = self.virt_alloc_end;
        debug!(
            "alloc_segment {:#x} -> {:#x} (pfn {:#x} + {:#x} pages)",
            start, segment.vend, segment.pfn, pages
        );
        Ok(segment)
    }

    fn alloc_page(&mut self, arch: &mut dyn ArchBootSetup) -> Result<DomainSegment> {
        let start = self.virt_alloc_end;
        let pfn = self.pfn_alloc_end;

        self.chk_alloc_pages(arch, 1)?;
        debug!("alloc_page {:#x} (pfn {:#x})", start, pfn);
        Ok(DomainSegment {
            vstart: start,
            vend: (start + arch.page_size()) - 1,
            pfn,
            addr: 0,
            size: 0,
            pages: 1,
        })
    }

    fn alloc_module(
        &mut self,
        arch: &mut dyn ArchBootSetup,
        buffer: &[u8],
    ) -> Result<DomainSegment> {
        let segment = self.alloc_segment(arch, 0, buffer.len() as u64)?;
        let slice = unsafe { slice::from_raw_parts_mut(segment.addr as *mut u8, buffer.len()) };
        copy(slice, buffer);
        Ok(segment)
    }

    fn alloc_padding_pages(&mut self, arch: &mut dyn ArchBootSetup, boundary: u64) -> Result<()> {
        if (boundary & (arch.page_size() - 1)) != 0 {
            return Err(Error::MemorySetupFailed);
        }

        if boundary < self.virt_alloc_end {
            return Err(Error::MemorySetupFailed);
        }
        let pages = (boundary - self.virt_alloc_end) / arch.page_size();
        self.chk_alloc_pages(arch, pages)?;
        Ok(())
    }

    fn chk_alloc_pages(&mut self, arch: &mut dyn ArchBootSetup, pages: u64) -> Result<()> {
        if pages > self.total_pages
            || self.pfn_alloc_end > self.total_pages
            || pages > self.total_pages - self.pfn_alloc_end
        {
            return Err(Error::MemorySetupFailed);
        }

        self.pfn_alloc_end += pages;
        self.virt_alloc_end += pages * arch.page_size();
        Ok(())
    }
}

pub trait ArchBootSetup {
    fn page_size(&mut self) -> u64;
    fn page_shift(&mut self) -> u64;

    fn alloc_p2m_segment(
        &mut self,
        setup: &mut BootSetup,
        image_info: &BootImageInfo,
    ) -> Result<DomainSegment>;

    fn alloc_page_tables(
        &mut self,
        setup: &mut BootSetup,
        image_info: &BootImageInfo,
    ) -> Result<DomainSegment>;

    fn setup_page_tables(&mut self, setup: &mut BootSetup, state: &mut BootState) -> Result<()>;

    fn setup_start_info(
        &mut self,
        setup: &mut BootSetup,
        state: &BootState,
        cmdline: &str,
    ) -> Result<()>;

    fn setup_shared_info(&mut self, setup: &mut BootSetup, shared_info_frame: u64) -> Result<()>;

    fn setup_hypercall_page(
        &mut self,
        setup: &mut BootSetup,
        image_info: &BootImageInfo,
    ) -> Result<()>;

    fn meminit(&mut self, setup: &mut BootSetup, total_pages: u64) -> Result<()>;
    fn bootlate(&mut self, setup: &mut BootSetup, state: &mut BootState) -> Result<()>;
    fn vcpu(&mut self, setup: &mut BootSetup, state: &mut BootState) -> Result<()>;
}
