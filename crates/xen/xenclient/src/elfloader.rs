use crate::boot::{BootImageInfo, BootImageLoader};
use crate::error::Result;
use crate::sys::{
    XEN_ELFNOTE_ENTRY, XEN_ELFNOTE_HYPERCALL_PAGE, XEN_ELFNOTE_INIT_P2M, XEN_ELFNOTE_MOD_START_PFN,
    XEN_ELFNOTE_PADDR_OFFSET, XEN_ELFNOTE_TYPES, XEN_ELFNOTE_VIRT_BASE,
};
use crate::Error;
use elf::abi::{PF_R, PF_W, PF_X, PT_LOAD, SHT_NOTE};
use elf::endian::AnyEndian;
use elf::note::Note;
use elf::ElfBytes;
use flate2::bufread::GzDecoder;
use log::debug;
use memchr::memmem::find_iter;
use slice_copy::copy;
use std::collections::HashMap;
use std::io::{BufReader, Read};
use std::mem::size_of;
use xz2::bufread::XzDecoder;

pub struct ElfImageLoader {
    data: Vec<u8>,
}

fn xen_note_value_as_u64(endian: AnyEndian, value: &[u8]) -> Option<u64> {
    let bytes = value.to_vec();
    match value.len() {
        1 => {
            let bytes: Option<[u8; size_of::<u8>()]> = bytes.try_into().ok();
            Some(match endian {
                AnyEndian::Little => u8::from_le_bytes(bytes?),
                AnyEndian::Big => u8::from_be_bytes(bytes?),
            } as u64)
        }
        2 => {
            let bytes: Option<[u8; size_of::<u16>()]> = bytes.try_into().ok();
            Some(match endian {
                AnyEndian::Little => u16::from_le_bytes(bytes?),
                AnyEndian::Big => u16::from_be_bytes(bytes?),
            } as u64)
        }
        4 => {
            let bytes: Option<[u8; size_of::<u32>()]> = bytes.try_into().ok();
            Some(match endian {
                AnyEndian::Little => u32::from_le_bytes(bytes?),
                AnyEndian::Big => u32::from_be_bytes(bytes?),
            } as u64)
        }
        8 => {
            let bytes: Option<[u8; size_of::<u64>()]> = bytes.try_into().ok();
            Some(match endian {
                AnyEndian::Little => u64::from_le_bytes(bytes?),
                AnyEndian::Big => u64::from_be_bytes(bytes?),
            })
        }
        _ => None,
    }
}

impl ElfImageLoader {
    pub fn new(data: Vec<u8>) -> ElfImageLoader {
        ElfImageLoader { data }
    }

    pub fn load_gz(data: &[u8]) -> Result<ElfImageLoader> {
        let buff = BufReader::new(data);
        let image = ElfImageLoader::read_one_stream(&mut GzDecoder::new(buff))?;
        Ok(ElfImageLoader::new(image))
    }

    pub fn load_xz(data: &[u8]) -> Result<ElfImageLoader> {
        let buff = BufReader::new(data);
        let image = ElfImageLoader::read_one_stream(&mut XzDecoder::new(buff))?;
        Ok(ElfImageLoader::new(image))
    }

    fn read_one_stream(read: &mut dyn Read) -> Result<Vec<u8>> {
        let mut result: Vec<u8> = Vec::new();
        let mut buffer = [0u8; 8192];

        loop {
            match read.read(&mut buffer) {
                Ok(size) => {
                    if size == 0 {
                        break;
                    }
                    result.extend_from_slice(&buffer[0..size])
                }
                Err(error) => {
                    if !result.is_empty() {
                        break;
                    }
                    return Err(Error::from(error));
                }
            }
        }
        Ok(result)
    }

    pub fn load_file_gz(path: &str) -> Result<ElfImageLoader> {
        let file = std::fs::read(path)?;
        ElfImageLoader::load_gz(file.as_slice())
    }

    pub fn load_file_xz(path: &str) -> Result<ElfImageLoader> {
        let file = std::fs::read(path)?;
        ElfImageLoader::load_xz(file.as_slice())
    }

    pub fn load_file_kernel(data: &[u8]) -> Result<ElfImageLoader> {
        for start in find_iter(data, &[0x1f, 0x8b]) {
            if let Ok(elf) = ElfImageLoader::load_gz(&data[start..]) {
                return Ok(elf);
            }
        }

        for start in find_iter(data, &[0xfd, 0x37, 0x7a, 0x58]) {
            if let Ok(elf) = ElfImageLoader::load_xz(&data[start..]) {
                return Ok(elf);
            }
        }

        Err(Error::ElfCompressionUnknown)
    }
}

struct ElfNoteValue {
    value: u64,
}

#[async_trait::async_trait]
impl BootImageLoader for ElfImageLoader {
    async fn parse(&self) -> Result<BootImageInfo> {
        let elf = ElfBytes::<AnyEndian>::minimal_parse(self.data.as_slice())?;
        let headers = elf.section_headers().ok_or(Error::ElfInvalidImage)?;
        let mut linux_notes: HashMap<u64, Vec<u8>> = HashMap::new();
        let mut xen_notes: HashMap<u64, ElfNoteValue> = HashMap::new();

        for header in headers {
            if header.sh_type != SHT_NOTE {
                continue;
            }

            let notes = elf.section_data_as_notes(&header)?;
            for note in notes {
                if let Note::Unknown(note) = note {
                    if note.name == "Linux" {
                        linux_notes.insert(note.n_type, note.desc.to_vec());
                    }

                    if note.name == "Xen" {
                        for typ in XEN_ELFNOTE_TYPES {
                            if typ.id != note.n_type {
                                continue;
                            }

                            let value = if !typ.is_string {
                                xen_note_value_as_u64(elf.ehdr.endianness, note.desc).unwrap_or(0)
                            } else {
                                0
                            };

                            xen_notes.insert(typ.id, ElfNoteValue { value });
                        }
                        continue;
                    }
                }
            }
        }

        if linux_notes.is_empty() {
            return Err(Error::ElfInvalidImage);
        }

        if xen_notes.is_empty() {
            return Err(Error::ElfXenSupportMissing);
        }

        let paddr_offset = xen_notes
            .get(&XEN_ELFNOTE_PADDR_OFFSET)
            .ok_or(Error::ElfInvalidImage)?
            .value;
        let virt_base = xen_notes
            .get(&XEN_ELFNOTE_VIRT_BASE)
            .ok_or(Error::ElfInvalidImage)?
            .value;
        let entry = xen_notes
            .get(&XEN_ELFNOTE_ENTRY)
            .ok_or(Error::ElfInvalidImage)?
            .value;
        let virt_hypercall = xen_notes
            .get(&XEN_ELFNOTE_HYPERCALL_PAGE)
            .ok_or(Error::ElfInvalidImage)?
            .value;
        let init_p2m = xen_notes
            .get(&XEN_ELFNOTE_INIT_P2M)
            .ok_or(Error::ElfInvalidImage)?
            .value;
        let mod_start_pfn = xen_notes
            .get(&XEN_ELFNOTE_MOD_START_PFN)
            .ok_or(Error::ElfInvalidImage)?
            .value;

        let mut start: u64 = u64::MAX;
        let mut end: u64 = 0;

        let segments = elf.segments().ok_or(Error::ElfInvalidImage)?;

        for header in segments {
            if (header.p_type != PT_LOAD) || (header.p_flags & (PF_R | PF_W | PF_X)) == 0 {
                continue;
            }
            let paddr = header.p_paddr;
            let memsz = header.p_memsz;
            if start > paddr {
                start = paddr;
            }

            if end < paddr + memsz {
                end = paddr + memsz;
            }
        }

        if paddr_offset != u64::MAX && virt_base == u64::MAX {
            return Err(Error::ElfInvalidImage);
        }

        let virt_offset = virt_base - paddr_offset;
        let virt_kstart = start + virt_offset;
        let virt_kend = end + virt_offset;
        let virt_entry = entry;

        let image_info = BootImageInfo {
            start,
            virt_base,
            virt_kstart,
            virt_kend,
            virt_hypercall,
            virt_entry,
            virt_p2m_base: init_p2m,
            unmapped_initrd: mod_start_pfn != 0,
        };
        Ok(image_info)
    }

    async fn load(&self, image_info: &BootImageInfo, dst: &mut [u8]) -> Result<()> {
        let elf = ElfBytes::<AnyEndian>::minimal_parse(self.data.as_slice())?;
        let segments = elf.segments().ok_or(Error::ElfInvalidImage)?;

        debug!(
            "load dst={:#x} segments={}",
            dst.as_ptr() as u64,
            segments.len()
        );
        for header in segments {
            let paddr = header.p_paddr;
            let filesz = header.p_filesz;
            let memsz = header.p_memsz;
            let base_offset = paddr - image_info.start;
            let data = elf.segment_data(&header)?;
            let segment_dst = &mut dst[base_offset as usize..];
            let copy_slice = &data[0..filesz as usize];
            debug!(
                "load copy hdr={:?} dst={:#x} len={}",
                header,
                copy_slice.as_ptr() as u64,
                copy_slice.len()
            );
            copy(segment_dst, copy_slice);
            if (memsz - filesz) > 0 {
                let remaining = &mut segment_dst[filesz as usize..memsz as usize];
                debug!(
                    "load fill_zero hdr={:?} dst={:#x} len={}",
                    header.p_offset,
                    remaining.as_ptr() as u64,
                    remaining.len()
                );
                remaining.fill(0);
            }
        }
        Ok(())
    }
}
