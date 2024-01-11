use crate::mem::PhysicalPages;
use crate::sys::{
    SUPERPAGE_2MB_NR_PFNS, SUPERPAGE_2MB_SHIFT, SUPERPAGE_BATCH_SIZE, XEN_PAGE_SHIFT,
};
use crate::XenClientError;
use libc::memset;
use std::ffi::c_void;
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
    memkb: u64,
    virt_alloc_end: u64,
    pfn_alloc_end: u64,
}

struct DomainSegment {
    _vstart: u64,
    _vend: u64,
    pfn: u64,
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
        memkb: u64,
    ) -> BootSetup<'a> {
        BootSetup {
            domctl,
            memctl,
            phys: PhysicalPages::new(call, domid),
            domid,
            memkb,
            virt_alloc_end: 0,
            pfn_alloc_end: 0,
        }
    }

    fn initialize_memory(&mut self) -> Result<(), XenClientError> {
        let mem_mb: u64 = self.memkb / 1024;
        let page_count: u64 = mem_mb << (20 - XEN_PAGE_SHIFT);
        let mut pfn_base_idx: u64 = 0;
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
            while super_pages > 0 {
                let count = super_pages.min(SUPERPAGE_BATCH_SIZE);
                super_pages -= count;

                for (i, pfn) in (pfn_base_idx..(count << SUPERPAGE_2MB_SHIFT))
                    .step_by(SUPERPAGE_2MB_NR_PFNS as usize)
                    .enumerate()
                {
                    extents[i] = p2m[pfn as usize];
                }

                let starts = self.memctl.populate_physmap(
                    self.domid,
                    count,
                    SUPERPAGE_2MB_SHIFT as u32,
                    0,
                    extents.as_slice(),
                )?;

                let pfn = pfn_base;
                for mfn in starts {
                    for k in 0..SUPERPAGE_2MB_NR_PFNS {
                        p2m[pfn as usize] = mfn + k;
                    }
                }
                pfn_base_idx = pfn;
            }

            let mut j = pfn_base_idx - pfn_base;

            loop {
                if j >= pages {
                    break;
                }

                let allocsz = (pages - j).min(1024 * 1024);
                let result = self.memctl.populate_physmap(
                    self.domid,
                    allocsz,
                    0,
                    0,
                    &[p2m[(pfn_base + j) as usize]],
                )?;
                p2m[(pfn_base + j) as usize] = result[0];
                j += allocsz;
            }
        }

        self.phys.load_p2m(p2m);
        Ok(())
    }

    fn initialize_hypercall(&mut self, image_info: BootImageInfo) -> Result<(), XenClientError> {
        if image_info.virt_hypercall != XEN_UNSET_ADDR {
            self.domctl
                .hypercall_init(self.domid, image_info.virt_hypercall)?;
        }
        Ok(())
    }

    pub fn initialize(&mut self, image_info: BootImageInfo) -> Result<(), XenClientError> {
        self.initialize_memory()?;
        let _kernel_segment = self.alloc_segment(image_info.virt_kend - image_info.virt_kstart)?;
        self.initialize_hypercall(image_info)?;
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
            _pages: pages,
        };
        let ptr = self.phys.pfn_to_ptr(segment.pfn, pages)?;
        unsafe {
            memset(ptr as *mut c_void, 0, (pages * page_size) as usize);
        }
        self.virt_alloc_end += pages * page_size;
        segment._vend = self.virt_alloc_end;
        self.pfn_alloc_end += 1;
        Ok(segment)
    }
}
