use std::mem::size_of;

use nix::{ioc, ioctl_readwrite_bad};

#[repr(C)]
#[derive(Clone, Debug)]
pub struct GrantRef {
    pub domid: u32,
    pub reference: u32,
}

pub struct MapGrantRef {
    pub count: u32,
    pub pad: u32,
    pub index: u64,
    pub refs: Vec<GrantRef>,
}

impl MapGrantRef {
    pub fn write(slice: &[GrantRef]) -> Vec<u32> {
        let mut values = vec![slice.len() as u32, 0, 0, 0];
        for r in slice {
            values.push(r.domid);
            values.push(r.reference);
        }
        values
    }

    pub fn read(count: u32, data: Vec<u32>) -> Option<MapGrantRef> {
        let mut refs = Vec::new();

        if data.len() < (4 + (count as u64 * 2)) as usize {
            return None;
        }

        let index = (*data.get(2)? as u64) | (*data.get(3)? as u64) << 32;
        for i in (4..data.len()).step_by(2) {
            let Some(domid) = data.get(i) else {
                break;
            };
            let Some(reference) = data.get(i + 1) else {
                break;
            };
            refs.push(GrantRef {
                domid: *domid,
                reference: *reference,
            });
        }

        Some(MapGrantRef {
            count,
            pad: 0,
            index,
            refs,
        })
    }
}

#[repr(C)]
pub struct UnmapGrantRef {
    pub index: u64,
    pub count: u32,
    pub pad: u32,
}

#[repr(C)]
pub struct GetOffsetForVaddr {
    pub vaddr: u64,
    pub offset: u64,
    pub count: u32,
    pub pad: u32,
}

#[repr(C)]
pub struct SetMaxGrants {
    pub count: u32,
}

#[repr(C)]
pub struct UnmapNotify {
    pub index: u64,
    pub action: u32,
    pub port: u32,
}

pub const UNMAP_NOTIFY_CLEAR_BYTE: u32 = 0x1;
pub const UNMAP_NOTIFY_SEND_EVENT: u32 = 0x2;

ioctl_readwrite_bad!(map_grant_ref, ioc!(nix::sys::ioctl::NONE, 'G', 0, 24), u32);
ioctl_readwrite_bad!(
    unmap_grant_ref,
    ioc!(nix::sys::ioctl::NONE, 'G', 1, size_of::<UnmapGrantRef>()),
    UnmapGrantRef
);
ioctl_readwrite_bad!(
    get_offset_for_vaddr,
    ioc!(
        nix::sys::ioctl::NONE,
        'G',
        2,
        size_of::<GetOffsetForVaddr>()
    ),
    GetOffsetForVaddr
);
ioctl_readwrite_bad!(
    set_max_grants,
    ioc!(nix::sys::ioctl::NONE, 'G', 3, size_of::<SetMaxGrants>()),
    SetMaxGrants
);
ioctl_readwrite_bad!(
    unmap_notify,
    ioc!(nix::sys::ioctl::NONE, 'G', 7, size_of::<UnmapNotify>()),
    UnmapNotify
);

#[repr(C)]
pub struct AllocGref {
    pub domid: u16,
    pub flags: u16,
    pub count: u32,
}

impl AllocGref {
    pub fn write(gref: AllocGref) -> Vec<u16> {
        let mut values = vec![
            gref.domid,
            gref.flags,
            (gref.count << 16) as u16,
            gref.count as u16,
            0,
            0,
            0,
            0,
        ];
        for _ in 0..gref.count {
            values.push(0);
            values.push(0);
        }
        values
    }

    pub fn read(count: u32, data: Vec<u16>) -> Option<(u64, Vec<u32>)> {
        let mut refs = Vec::new();

        if data.len() < (8 + (count as u64 * 2)) as usize {
            return None;
        }

        let index = (*data.get(4)? as u64)
            | (*data.get(5)? as u64) << 16
            | (*data.get(6)? as u64) << 32
            | (*data.get(7)? as u64) << 48;
        for i in (8..data.len()).step_by(2) {
            let Some(bits_low) = data.get(i) else {
                break;
            };
            let Some(bits_high) = data.get(i + 1) else {
                break;
            };
            refs.push((*bits_low as u32) | (*bits_high as u32) << 16);
        }
        Some((index, refs))
    }
}

#[repr(C)]
pub struct DeallocGref {
    pub index: u64,
    pub count: u32,
}

ioctl_readwrite_bad!(alloc_gref, ioc!(nix::sys::ioctl::NONE, 'G', 5, 20), u16);
ioctl_readwrite_bad!(
    dealloc_gref,
    ioc!(nix::sys::ioctl::NONE, 'G', 6, size_of::<DeallocGref>()),
    DeallocGref
);
