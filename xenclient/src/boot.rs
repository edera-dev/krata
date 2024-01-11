use crate::mem::PhysicalPages;
use crate::sys::{
    SUPERPAGE_2MB_NR_PFNS, SUPERPAGE_2MB_SHIFT, SUPERPAGE_BATCH_SIZE, XEN_PAGE_SHIFT,
};
use crate::XenClientError;
use libc::memset;
use log::debug;
use std::ffi::c_void;
use std::slice;
use xencall::domctl::DomainControl;
use xencall::memory::MemoryControl;
use xencall::XenCall;

pub trait BootImageLoader {
    fn parse(&self) -> Result<BootImageInfo, XenClientError>;
    fn load(&self, image_info: BootImageInfo, dst: &mut [u8]) -> Result<(), XenClientError>;
}

pub const XEN_UNSET_ADDR: u64 = -1i64 as u64;

#[derive(Debug)]
pub struct BootImageInfo {
    pub virt_kstart: u64,
    pub virt_kend: u64,
    pub virt_hypercall: u64,
    pub virt_entry: u64,
    pub init_p2m: u64,
}

pub struct BootSetup<'a> {
    domctl: &'a DomainControl<'a>,
    memctl: &'a MemoryControl<'a>,
    phys: PhysicalPages<'a>,
    domid: u32,
    virt_alloc_end: u64,
    pfn_alloc_end: u64,
}

#[derive(Debug)]
struct DomainSegment {
    _vstart: u64,
    _vend: u64,
    pfn: u64,
    addr: u64,
    size: u64,
    _pages: u64,
}

struct VmemRange {
    start: u64,
    end: u64,
    _flags: u32,
    _nid: u32,
}

impl BootSetup<'_> {
    pub fn new<'a>(
        call: &'a XenCall,
        domctl: &'a DomainControl<'a>,
        memctl: &'a MemoryControl<'a>,
        domid: u32,
    ) -> BootSetup<'a> {
        BootSetup {
            domctl,
            memctl,
            phys: PhysicalPages::new(call, domid),
            domid,
            virt_alloc_end: 0,
            pfn_alloc_end: 0,
        }
    }

    fn initialize_memory(&mut self, memkb: u64) -> Result<(), XenClientError> {
        let mem_mb: u64 = memkb / 1024;
        let page_count: u64 = mem_mb << (20 - XEN_PAGE_SHIFT);
        let mut vmemranges: Vec<VmemRange> = Vec::new();
        let stub = VmemRange {
            start: 0,
            end: page_count << XEN_PAGE_SHIFT,
            _flags: 0,
            _nid: 0,
        };
        vmemranges.push(stub);

        let mut p2m_size: u64 = 0;
        let mut total: u64 = 0;
        for range in &vmemranges {
            total += (range.end - range.start) >> XEN_PAGE_SHIFT;
            p2m_size = p2m_size.max(range.end >> XEN_PAGE_SHIFT);
        }

        if total != page_count {
            return Err(XenClientError::new(
                "Page count mismatch while calculating pages.",
            ));
        }

        let mut p2m = vec![-1i64 as u64; p2m_size as usize];
        for range in &vmemranges {
            let mut extents = vec![0u64; SUPERPAGE_BATCH_SIZE as usize];
            let pages = (range.end - range.start) >> XEN_PAGE_SHIFT;
            let pfn_base = range.start >> XEN_PAGE_SHIFT;

            for pfn in pfn_base..pfn_base + pages {
                p2m[pfn as usize] = pfn;
            }

            let mut super_pages = pages >> SUPERPAGE_2MB_SHIFT;
            let mut pfn_base_idx: u64 = pfn_base;
            while super_pages > 0 {
                let count = super_pages.min(SUPERPAGE_BATCH_SIZE);
                super_pages -= count;

                let mut j: usize = 0;
                let mut pfn: u64 = pfn_base_idx;
                loop {
                    if pfn >= pfn_base_idx + (count << SUPERPAGE_2MB_SHIFT) {
                        break;
                    }
                    extents[j] = p2m[pfn as usize];
                    pfn += SUPERPAGE_2MB_NR_PFNS;
                    j += 1;
                }

                let starts = self.memctl.populate_physmap(
                    self.domid,
                    count,
                    SUPERPAGE_2MB_SHIFT as u32,
                    0,
                    extents.as_slice(),
                )?;

                pfn = pfn_base_idx;
                for mfn in starts {
                    for k in 0..SUPERPAGE_2MB_NR_PFNS {
                        p2m[pfn as usize] = mfn + k;
                        pfn += 1;
                    }
                }
                pfn_base_idx = pfn;
            }

            let mut j = pfn_base_idx - pfn_base;
            loop {
                if j >= pages {
                    break;
                }

                let allocsz = (1024 * 1024).min(pages - j);
                let p2m_idx = (pfn_base + j) as usize;
                let p2m_end_idx = p2m_idx + allocsz as usize;
                let result = self.memctl.populate_physmap(
                    self.domid,
                    allocsz,
                    0,
                    0,
                    &p2m[p2m_idx..p2m_end_idx],
                )?;

                if result.len() != allocsz as usize {
                    return Err(XenClientError::new(
                        format!("failed to populate physmap: {:?}", result).as_str(),
                    ));
                }

                p2m[p2m_idx] = result[0];
                j += allocsz;
            }
        }

        self.phys.load_p2m(p2m);
        Ok(())
    }

    fn _initialize_hypercall(&mut self, image_info: BootImageInfo) -> Result<(), XenClientError> {
        if image_info.virt_hypercall != XEN_UNSET_ADDR {
            self.domctl
                .hypercall_init(self.domid, image_info.virt_hypercall)?;
        }
        Ok(())
    }

    pub fn initialize(
        &mut self,
        image_loader: &dyn BootImageLoader,
        memkb: u64,
    ) -> Result<(), XenClientError> {
        debug!("BootSetup initialize memkb={:?}", memkb);
        let image_info = image_loader.parse()?;
        debug!("BootSetup initialize image_info={:?}", image_info);
        self.domctl.set_max_mem(self.domid, memkb)?;
        self.initialize_memory(memkb)?;
        let kernel_segment = self.alloc_segment(image_info.virt_kend - image_info.virt_kstart)?;
        let kernel_segment_ptr = kernel_segment.addr as *mut u8;
        debug!(
            "BootSetup initialize kernel_segment ptr={:#x}",
            kernel_segment_ptr as u64
        );
        let slice =
            unsafe { slice::from_raw_parts_mut(kernel_segment_ptr, kernel_segment.size as usize) };
        image_loader.load(image_info, slice)?;
        Ok(())
    }

    fn alloc_segment(&mut self, size: u64) -> Result<DomainSegment, XenClientError> {
        let page_size = 1u64 << XEN_PAGE_SHIFT;
        let pages = (size + page_size - 1) / page_size;
        let start = self.virt_alloc_end;
        let mut segment = DomainSegment {
            _vstart: start,
            _vend: 0,
            pfn: self.pfn_alloc_end,
            addr: 0,
            size,
            _pages: pages,
        };
        let ptr = self.phys.pfn_to_ptr(segment.pfn, pages)?;
        segment.addr = ptr;
        unsafe {
            memset(ptr as *mut c_void, 0, (pages * page_size) as usize);
        }
        self.virt_alloc_end += pages * page_size;
        segment._vend = self.virt_alloc_end;
        self.pfn_alloc_end += 1;
        debug!("BootSetup alloc_segment size={} ptr={:#x}", size, ptr);
        Ok(segment)
    }
}
