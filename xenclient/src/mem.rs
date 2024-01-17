use crate::sys::XEN_PAGE_SHIFT;
use crate::XenClientError;
use libc::munmap;
use log::debug;
use std::ffi::c_void;

use crate::x86::X86_PAGE_SHIFT;
use xencall::sys::MmapEntry;
use xencall::XenCall;

#[derive(Debug)]
pub struct PhysicalPage {
    pfn: u64,
    ptr: u64,
    count: u64,
}

pub struct PhysicalPages<'a> {
    domid: u32,
    pub(crate) p2m: Vec<u64>,
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

    pub fn p2m_size(&mut self) -> u64 {
        self.p2m.len() as u64
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

            return Ok(page.ptr + ((pfn - page.pfn) << X86_PAGE_SHIFT));
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
        for (i, entry) in entries.iter_mut().enumerate() {
            entry.mfn = self.p2m[pfn as usize + i];
        }
        let chunk_size = 1 << XEN_PAGE_SHIFT;
        let num_per_entry = chunk_size >> XEN_PAGE_SHIFT;
        let num = num_per_entry * count as usize;
        let mut pfns = vec![u64::MAX; num];
        for i in 0..count as usize {
            for j in 0..num_per_entry {
                pfns[i * num_per_entry + j] = entries[i].mfn + j as u64;
            }
        }

        let actual_mmap_len = (num as u64) << XEN_PAGE_SHIFT;
        let addr = self
            .call
            .mmap(0, actual_mmap_len)
            .ok_or(XenClientError::new("failed to mmap address"))?;
        debug!("mapped {:#x} foreign bytes at {:#x}", actual_mmap_len, addr);
        let result = self.call.mmap_batch(self.domid, num as u64, addr, pfns)?;
        if result != 0 {
            return Err(XenClientError::new("mmap_batch call failed"));
        }
        let page = PhysicalPage {
            pfn,
            ptr: addr,
            count,
        };
        debug!(
            "alloc_pfn {:#x}+{:#x} at {:#x}",
            page.pfn, page.count, page.ptr
        );
        self.pages.push(page);
        Ok(addr)
    }

    pub fn unmap_all(&mut self) -> Result<(), XenClientError> {
        for page in &self.pages {
            unsafe {
                let err = munmap(
                    page.ptr as *mut c_void,
                    (page.count << X86_PAGE_SHIFT) as usize,
                );
                if err != 0 {
                    return Err(XenClientError::new("failed to munmap all pages"));
                }
            }
        }
        self.pages.clear();
        Ok(())
    }

    pub fn unmap(&mut self, pfn: u64) -> Result<(), XenClientError> {
        let page = self.pages.iter().find(|x| x.pfn == pfn);
        if page.is_none() {
            return Err(XenClientError::new("unable to find page to unmap"));
        }
        let page = page.unwrap();

        unsafe {
            let err = munmap(
                page.ptr as *mut c_void,
                (page.count << X86_PAGE_SHIFT) as usize,
            );
            debug!(
                "unmapped {:#x} foreign bytes at {:#x}",
                (page.count << X86_PAGE_SHIFT) as usize,
                page.ptr
            );
            if err != 0 {
                return Err(XenClientError::new("failed to munmap page"));
            }
        }
        Ok(())
    }
}
