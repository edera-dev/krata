use crate::sys::XEN_PAGE_SHIFT;
use crate::XenClientError;

use xencall::sys::MmapEntry;
use xencall::XenCall;

pub struct PhysicalPage {
    pfn: u64,
    ptr: u64,
    count: u64,
}

pub struct PhysicalPages<'a> {
    domid: u32,
    p2m: Vec<u64>,
    call: &'a XenCall,
    pages: Vec<PhysicalPage>,
}

impl PhysicalPages<'_> {
    pub fn new(call: &XenCall, domid: u32) -> PhysicalPages {
        PhysicalPages {
            domid,
            p2m: Vec::new(),
            call,
            pages: Vec::new(),
        }
    }

    pub fn load_p2m(&mut self, p2m: Vec<u64>) {
        self.p2m = p2m;
    }

    pub fn pfn_to_ptr(&mut self, pfn: u64, count: u64) -> Result<u64, XenClientError> {
        for page in &self.pages {
            if pfn >= page.pfn + page.count {
                continue;
            }

            if count > 0 {
                if (pfn + count) <= page.pfn {
                    continue;
                }

                if pfn < page.pfn || (pfn + count) > page.pfn + page.count {
                    return Err(XenClientError::new("request overlaps allocated block"));
                }
            } else {
                if pfn < page.pfn {
                    continue;
                }

                if pfn >= page.pfn + page.count {
                    continue;
                }
            }

            return Ok(page.ptr + ((pfn - page.pfn) << XEN_PAGE_SHIFT));
        }

        if count == 0 {
            return Err(XenClientError::new(
                "allocation is only allowed when a size is given",
            ));
        }

        self.pfn_alloc(pfn, count)
    }

    fn pfn_alloc(&mut self, pfn: u64, count: u64) -> Result<u64, XenClientError> {
        let mut entries = vec![MmapEntry::default(); count as usize];
        for (i, entry) in (0_u64..).zip(entries.iter_mut()) {
            entry.mfn = self.p2m[(pfn + i) as usize];
        }
        let chunk_size = 1 << XEN_PAGE_SHIFT;
        let num_per_entry = chunk_size >> XEN_PAGE_SHIFT;
        let num = num_per_entry * entries.len();
        let mut pfns = vec![0u64; num];
        for i in 0..entries.len() {
            for j in 0..num_per_entry {
                pfns[i * num_per_entry + j] = entries[i].mfn + j as u64;
            }
        }

        let size = count << XEN_PAGE_SHIFT;
        let addr = self
            .call
            .mmap(0, size)
            .ok_or(XenClientError::new("failed to mmap address"))?;
        self.call.mmap_batch(self.domid, num as u64, addr, pfns)?;
        let page = PhysicalPage {
            pfn,
            ptr: addr,
            count,
        };
        self.pages.push(page);
        Ok(addr)
    }
}
