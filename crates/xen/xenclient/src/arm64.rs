use crate::boot::{ArchBootSetup, BootImageInfo, BootSetup, BootState, DomainSegment};
use crate::error::Result;
use crate::sys::XEN_PAGE_SHIFT;
use crate::Error;
use log::trace;
use xencall::sys::VcpuGuestContext;

pub const ARM_PAGE_SHIFT: u64 = 12;
const ARM_PAGE_SIZE: u64 = 1 << ARM_PAGE_SHIFT;

const GUEST_RAM0_BASE: u64 = 0x40000000;
const GUEST_RAM0_SIZE: u64 = 0xc0000000;
const GUEST_RAM1_BASE: u64 = 0x0200000000;
const GUEST_RAM1_SIZE: u64 = 0xfe00000000;

const GUEST_RAM_BANK_BASES: [u64; 2] = [GUEST_RAM0_BASE, GUEST_RAM1_BASE];
const GUEST_RAM_BANK_SIZES: [u64; 2] = [GUEST_RAM0_SIZE, GUEST_RAM1_SIZE];

const LPAE_SHIFT: u64 = 9;
const PFN_4K_SHIFT: u64 = 0;
const PFN_2M_SHIFT: u64 = PFN_4K_SHIFT + LPAE_SHIFT;
const PFN_1G_SHIFT: u64 = PFN_2M_SHIFT + LPAE_SHIFT;
const PFN_512G_SHIFT: u64 = PFN_1G_SHIFT + LPAE_SHIFT;

const PSR_FIQ_MASK: u64 = 1 << 6; /* Fast Interrupt mask */
const PSR_IRQ_MASK: u64 = 1 << 7; /* Interrupt mask */
const PSR_ABT_MASK: u64 = 1 << 8; /* Asynchronous Abort mask */
const PSR_MODE_EL1H: u64 = 0x05;
const PSR_GUEST64_INIT: u64 = PSR_ABT_MASK | PSR_FIQ_MASK | PSR_IRQ_MASK | PSR_MODE_EL1H;

pub struct Arm64BootSetup {}

impl Default for Arm64BootSetup {
    fn default() -> Self {
        Self::new()
    }
}

impl Arm64BootSetup {
    pub fn new() -> Arm64BootSetup {
        Arm64BootSetup {}
    }

    fn populate_one_size(
        &mut self,
        setup: &mut BootSetup,
        pfn_shift: u64,
        base_pfn: u64,
        pfn_count: u64,
        extents: &mut [u64],
    ) -> Result<u64> {
        let mask = (1u64 << pfn_shift) - 1;
        let next_shift = pfn_shift + LPAE_SHIFT;
        let next_mask = (1u64 << next_shift) - 1;
        let next_boundary = (base_pfn + (1 << next_shift)) - 1;

        let mut end_pfn = base_pfn + pfn_count;

        if pfn_shift == PFN_512G_SHIFT {
            return Ok(0);
        }

        if (base_pfn & next_mask) != 0 && end_pfn > next_boundary {
            end_pfn = next_boundary;
        }

        if (mask & base_pfn) != 0 {
            return Ok(0);
        }

        let count = (end_pfn - base_pfn) >> pfn_shift;

        if count == 0 {
            return Ok(0);
        }

        for i in 0..count {
            extents[i as usize] = base_pfn + (i << pfn_shift);
        }

        let result_extents = setup.call.populate_physmap(
            setup.domid,
            count,
            pfn_shift as u32,
            0,
            &extents[0usize..count as usize],
        )?;
        slice_copy::copy(extents, &result_extents);
        Ok((result_extents.len() as u64) << pfn_shift)
    }

    fn populate_guest_memory(
        &mut self,
        setup: &mut BootSetup,
        base_pfn: u64,
        pfn_count: u64,
    ) -> Result<()> {
        let mut extents = vec![0u64; 1024 * 1024];

        for pfn in 0..extents.len() {
            let mut allocsz = (1024 * 1024).min(pfn_count - pfn as u64);
            allocsz = self.populate_one_size(
                setup,
                PFN_512G_SHIFT,
                base_pfn + pfn as u64,
                allocsz,
                &mut extents,
            )?;
            if allocsz > 0 {
                continue;
            }
            allocsz = self.populate_one_size(
                setup,
                PFN_1G_SHIFT,
                base_pfn + pfn as u64,
                allocsz,
                &mut extents,
            )?;
            if allocsz > 0 {
                continue;
            }
            allocsz = self.populate_one_size(
                setup,
                PFN_2M_SHIFT,
                base_pfn + pfn as u64,
                allocsz,
                &mut extents,
            )?;
            if allocsz > 0 {
                continue;
            }
            allocsz = self.populate_one_size(
                setup,
                PFN_4K_SHIFT,
                base_pfn + pfn as u64,
                allocsz,
                &mut extents,
            )?;
            if allocsz == 0 {
                return Err(Error::MemorySetupFailed("allocsz is zero"));
            }
        }

        Ok(())
    }
}

impl ArchBootSetup for Arm64BootSetup {
    fn page_size(&mut self) -> u64 {
        ARM_PAGE_SIZE
    }

    fn page_shift(&mut self) -> u64 {
        ARM_PAGE_SHIFT
    }

    fn needs_early_kernel(&mut self) -> bool {
        true
    }

    fn setup_shared_info(&mut self, _: &mut BootSetup, _: u64) -> Result<()> {
        Ok(())
    }

    fn setup_start_info(&mut self, _: &mut BootSetup, _: &BootState, _: &str) -> Result<()> {
        Ok(())
    }

    fn meminit(
        &mut self,
        setup: &mut BootSetup,
        total_pages: u64,
        kernel_segment: &Option<DomainSegment>,
        initrd_segment: &Option<DomainSegment>,
    ) -> Result<()> {
        let kernel_segment = kernel_segment
            .as_ref()
            .ok_or(Error::MemorySetupFailed("kernel_segment missing"))?;
        setup.call.claim_pages(setup.domid, total_pages)?;
        let mut ramsize = total_pages << XEN_PAGE_SHIFT;

        let bankbase = GUEST_RAM_BANK_BASES;
        let bankmax = GUEST_RAM_BANK_SIZES;

        let kernbase = kernel_segment.vstart;
        let kernend = BootSetup::round_up(kernel_segment.size, 21);
        let dtb = setup.dtb.as_ref();
        let dtb_size = dtb.map(|blob| BootSetup::round_up(blob.len() as u64, XEN_PAGE_SHIFT));
        let ramdisk_size = initrd_segment
            .as_ref()
            .map(|segment| BootSetup::round_up(segment.size, XEN_PAGE_SHIFT));
        let modsize = dtb_size.unwrap_or(0) + ramdisk_size.unwrap_or(0);
        let ram128mb = bankbase[0] + (128 << 20);

        let mut rambank_size: [u64; 2] = [0, 0];
        for i in 0..2 {
            let size = if ramsize > bankmax[i] {
                bankmax[i]
            } else {
                ramsize
            };
            ramsize -= size;
            rambank_size[i] = size >> XEN_PAGE_SHIFT;
        }

        for i in 0..2 {
            let size = if ramsize > bankmax[i] {
                bankmax[i]
            } else {
                ramsize
            };
            ramsize -= size;
            rambank_size[i] = size >> XEN_PAGE_SHIFT;
        }

        for i in 0..2 {
            self.populate_guest_memory(setup, bankbase[i] >> XEN_PAGE_SHIFT, rambank_size[i])?;
        }

        let bank0end = bankbase[0] + (rambank_size[0] << XEN_PAGE_SHIFT);
        let _modbase = if bank0end >= ram128mb + modsize && kernend < ram128mb {
            ram128mb
        } else if bank0end - modsize > kernend {
            bank0end - modsize
        } else if kernbase - bankbase[0] > modsize {
            kernbase - modsize
        } else {
            return Err(Error::MemorySetupFailed("unable to determine modbase"));
        };
        setup.call.claim_pages(setup.domid, 0)?;
        Ok(())
    }

    fn bootlate(&mut self, _: &mut BootSetup, _: &mut BootState) -> Result<()> {
        Ok(())
    }

    fn vcpu(&mut self, setup: &mut BootSetup, state: &mut BootState) -> Result<()> {
        let mut vcpu = VcpuGuestContext::default();
        vcpu.user_regs.pc = state.image_info.virt_entry;
        vcpu.user_regs.x0 = 0xffffffff;
        vcpu.user_regs.x1 = 0;
        vcpu.user_regs.x2 = 0;
        vcpu.user_regs.x3 = 0;
        vcpu.sctlr = 0x00c50078;
        vcpu.ttbr0 = 0;
        vcpu.ttbr1 = 0;
        vcpu.ttbcr = 0;
        vcpu.user_regs.cpsr = PSR_GUEST64_INIT;
        vcpu.flags = 1 << 0; // VGCF_ONLINE
        trace!("vcpu context: {:?}", vcpu);
        setup.call.set_vcpu_context(setup.domid, 0, &vcpu)?;
        Ok(())
    }

    fn alloc_p2m_segment(
        &mut self,
        _: &mut BootSetup,
        _: &BootImageInfo,
    ) -> Result<Option<DomainSegment>> {
        Ok(None)
    }

    fn alloc_page_tables(
        &mut self,
        _: &mut BootSetup,
        _: &BootImageInfo,
    ) -> Result<Option<DomainSegment>> {
        Ok(None)
    }

    fn setup_page_tables(&mut self, _: &mut BootSetup, _: &mut BootState) -> Result<()> {
        Ok(())
    }
}
