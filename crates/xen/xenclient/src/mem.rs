use crate::error::Result;
use crate::sys::{XEN_PAGE_SHIFT, XEN_PAGE_SIZE};
use crate::Error;
use libc::{memset, munmap};
use log::debug;
use nix::errno::Errno;
use std::ffi::c_void;
use std::slice;

use xencall::sys::MmapEntry;
use xencall::XenCall;

#[derive(Debug, Clone)]
pub struct PhysicalPage {
    pfn: u64,
    pub ptr: u64,
    count: u64,
}

pub struct PhysicalPages {
    page_shift: u64,
    domid: u32,
    pub p2m: Vec<u64>,
    call: XenCall,
    pages: Vec<PhysicalPage>,
}

impl PhysicalPages {
    pub fn new(call: XenCall, domid: u32, page_shift: u64) -> PhysicalPages {
        PhysicalPages {
            page_shift,
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

    pub async fn pfn_to_ptr(&mut self, pfn: u64, count: u64) -> Result<u64> {
        for page in &self.pages {
            if pfn >= page.pfn + page.count {
                continue;
            }

            if count > 0 {
                if (pfn + count) <= page.pfn {
                    continue;
                }

                if pfn < page.pfn || (pfn + count) > page.pfn + page.count {
                    return Err(Error::MemorySetupFailed("pfn is out of range"));
                }
            } else {
                if pfn < page.pfn {
                    continue;
                }

                if pfn >= page.pfn + page.count {
                    continue;
                }
            }

            return Ok(page.ptr + ((pfn - page.pfn) << self.page_shift));
        }

        if count == 0 {
            return Err(Error::MemorySetupFailed("page count is zero"));
        }

        self.pfn_alloc(pfn, count).await
    }

    async fn pfn_alloc(&mut self, pfn: u64, count: u64) -> Result<u64> {
        let mut entries = vec![MmapEntry::default(); count as usize];
        for (i, entry) in entries.iter_mut().enumerate() {
            if !self.p2m.is_empty() {
                entry.mfn = self.p2m[pfn as usize + i];
            } else {
                entry.mfn = pfn + i as u64;
            }
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
            .await
            .ok_or(Error::MmapFailed)?;
        debug!("mapped {:#x} foreign bytes at {:#x}", actual_mmap_len, addr);
        let result = self
            .call
            .mmap_batch(self.domid, num as u64, addr, pfns)
            .await?;
        if result != 0 {
            return Err(Error::MmapFailed);
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

    pub async fn map_foreign_pages(&mut self, mfn: u64, size: u64) -> Result<PhysicalPage> {
        let num = (size >> XEN_PAGE_SHIFT) as usize;
        let mut pfns = vec![u64::MAX; num];
        for (i, item) in pfns.iter_mut().enumerate().take(num) {
            *item = mfn + i as u64;
        }

        let actual_mmap_len = (num as u64) << XEN_PAGE_SHIFT;
        let addr = self
            .call
            .mmap(0, actual_mmap_len)
            .await
            .ok_or(Error::MmapFailed)?;
        debug!("mapped {:#x} foreign bytes at {:#x}", actual_mmap_len, addr);
        let result = self
            .call
            .mmap_batch(self.domid, num as u64, addr, pfns)
            .await?;
        if result != 0 {
            return Err(Error::MmapFailed);
        }
        let page = PhysicalPage {
            pfn: mfn,
            ptr: addr,
            count: num as u64,
        };
        debug!(
            "alloc_mfn {:#x}+{:#x} at {:#x}",
            page.pfn, page.count, page.ptr
        );
        self.pages.push(page.clone());
        Ok(page)
    }

    pub async fn clear_pages(&mut self, pfn: u64, count: u64) -> Result<()> {
        let mfn = if !self.p2m.is_empty() {
            self.p2m[pfn as usize]
        } else {
            pfn
        };
        let page = self.map_foreign_pages(mfn, count << XEN_PAGE_SHIFT).await?;
        let slice = unsafe { slice::from_raw_parts_mut(page.ptr as *mut u8, (count << XEN_PAGE_SHIFT) as usize) };
        slice.fill(0);
        Ok(())
    }

    pub fn unmap_all(&mut self) -> Result<()> {
        for page in &self.pages {
            unsafe {
                let err = munmap(
                    page.ptr as *mut c_void,
                    (page.count << self.page_shift) as usize,
                );
                if err != 0 {
                    return Err(Error::UnmapFailed(Errno::from_raw(err)));
                }
            }
        }
        self.pages.clear();
        Ok(())
    }

    pub fn unmap(&mut self, pfn: u64) -> Result<()> {
        let page = self.pages.iter().enumerate().find(|(_, x)| x.pfn == pfn);
        if page.is_none() {
            return Err(Error::MemorySetupFailed("cannot unmap missing page"));
        }
        let (i, page) = page.unwrap();

        unsafe {
            let err = munmap(
                page.ptr as *mut c_void,
                (page.count << self.page_shift) as usize,
            );
            debug!(
                "unmapped {:#x} foreign bytes at {:#x}",
                (page.count << self.page_shift) as usize,
                page.ptr
            );
            if err != 0 {
                return Err(Error::UnmapFailed(Errno::from_raw(err)));
            }
            self.pages.remove(i);
        }
        Ok(())
    }
}
