use crate::boot::{BootImageInfo, BootImageLoader, XEN_UNSET_ADDR};
use crate::sys::{
    XEN_ELFNOTE_ENTRY, XEN_ELFNOTE_HYPERCALL_PAGE, XEN_ELFNOTE_INIT_P2M, XEN_ELFNOTE_PADDR_OFFSET,
    XEN_ELFNOTE_TYPES, XEN_ELFNOTE_VIRT_BASE,
};
use crate::XenClientError;
use elf::abi::{PF_R, PF_W, PF_X, PT_LOAD, SHT_NOTE};
use elf::endian::AnyEndian;
use elf::note::Note;
use elf::{ElfBytes, ParseError};
use flate2::bufread::GzDecoder;
use log::debug;
use memchr::memmem::find_iter;
use slice_copy::copy;
use std::collections::HashMap;
use std::ffi::{FromVecWithNulError, IntoStringError};
use std::io::{BufReader, Read};
use std::mem::size_of;
use xz2::bufread::XzDecoder;

impl From<ParseError> for XenClientError {
    fn from(value: ParseError) -> Self {
        XenClientError::new(value.to_string().as_str())
    }
}

impl From<FromVecWithNulError> for XenClientError {
    fn from(value: FromVecWithNulError) -> Self {
        XenClientError::new(value.to_string().as_str())
    }
}

impl From<IntoStringError> for XenClientError {
    fn from(value: IntoStringError) -> Self {
        XenClientError::new(value.to_string().as_str())
    }
}

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

    pub fn load_file(path: &str) -> Result<ElfImageLoader, XenClientError> {
        let data = std::fs::read(path)?;
        Ok(ElfImageLoader::new(data))
    }

    pub fn load_gz(data: &[u8]) -> Result<ElfImageLoader, XenClientError> {
        let buff = BufReader::new(data);
        let image = ElfImageLoader::read_one_stream(&mut GzDecoder::new(buff))?;
        Ok(ElfImageLoader::new(image))
    }

    pub fn load_xz(data: &[u8]) -> Result<ElfImageLoader, XenClientError> {
        let buff = BufReader::new(data);
        let image = ElfImageLoader::read_one_stream(&mut XzDecoder::new(buff))?;
        Ok(ElfImageLoader::new(image))
    }

    fn read_one_stream(read: &mut dyn Read) -> Result<Vec<u8>, XenClientError> {
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
                    return Err(XenClientError::from(error));
                }
            }
        }
        Ok(result)
    }

    pub fn load_file_gz(path: &str) -> Result<ElfImageLoader, XenClientError> {
        let file = std::fs::read(path)?;
        ElfImageLoader::load_gz(file.as_slice())
    }

    pub fn load_file_xz(path: &str) -> Result<ElfImageLoader, XenClientError> {
        let file = std::fs::read(path)?;
        ElfImageLoader::load_xz(file.as_slice())
    }

    pub fn load_file_kernel(path: &str) -> Result<ElfImageLoader, XenClientError> {
        let file = std::fs::read(path)?;

        for start in find_iter(file.as_slice(), &[0x1f, 0x8b]) {
            if let Ok(elf) = ElfImageLoader::load_gz(&file[start..]) {
                return Ok(elf);
            }
        }

        for start in find_iter(file.as_slice(), &[0xfd, 0x37, 0x7a, 0x58]) {
            if let Ok(elf) = ElfImageLoader::load_xz(&file[start..]) {
                return Ok(elf);
            }
        }

        Err(XenClientError::new(
            "Unable to parse kernel image: unknown compression type",
        ))
    }
}

struct ElfNoteValue {
    value: u64,
}

impl BootImageLoader for ElfImageLoader {
    fn parse(&self) -> Result<BootImageInfo, XenClientError> {
        let elf = ElfBytes::<AnyEndian>::minimal_parse(self.data.as_slice())?;
        let headers = elf.section_headers().ok_or(XenClientError::new(
            "Unable to parse kernel image: section headers not found.",
        ))?;
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
            return Err(XenClientError::new(
                "Provided kernel does not appear to be a Linux kernel image.",
            ));
        }

        if xen_notes.is_empty() {
            return Err(XenClientError::new(
                "Provided kernel does not have Xen support.",
            ));
        }

        let paddr_offset = xen_notes
            .get(&XEN_ELFNOTE_PADDR_OFFSET)
            .ok_or(XenClientError::new(
                "Unable to find paddr_offset note in kernel.",
            ))?
            .value;
        let virt_base = xen_notes
            .get(&XEN_ELFNOTE_VIRT_BASE)
            .ok_or(XenClientError::new(
                "Unable to find virt_base note in kernel.",
            ))?
            .value;
        let entry = xen_notes
            .get(&XEN_ELFNOTE_ENTRY)
            .ok_or(XenClientError::new("Unable to find entry note in kernel."))?
            .value;
        let virt_hypercall = xen_notes
            .get(&XEN_ELFNOTE_HYPERCALL_PAGE)
            .ok_or(XenClientError::new(
                "Unable to find hypercall_page note in kernel.",
            ))?
            .value;
        let init_p2m = xen_notes
            .get(&XEN_ELFNOTE_INIT_P2M)
            .ok_or(XenClientError::new(
                "Unable to find init_p2m note in kernel.",
            ))?
            .value;

        let mut start: u64 = u64::MAX;
        let mut end: u64 = 0;

        let segments = elf.segments().ok_or(XenClientError::new(
            "Unable to parse kernel image: segments not found.",
        ))?;

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

        if paddr_offset != XEN_UNSET_ADDR && virt_base == XEN_UNSET_ADDR {
            return Err(XenClientError::new(
                "Unable to load kernel image: paddr_offset set but virt_base is unset.",
            ));
        }

        let virt_base = 0;

        let _paddr_offset = if paddr_offset == XEN_UNSET_ADDR {
            0
        } else {
            paddr_offset
        };

        let virt_offset = 0;
        let virt_kstart = start + virt_offset;
        let virt_kend = end + virt_offset;
        let virt_entry = if entry == XEN_UNSET_ADDR {
            elf.ehdr.e_entry
        } else {
            entry
        };

        Ok(BootImageInfo {
            virt_base,
            virt_kstart,
            virt_kend,
            virt_hypercall,
            virt_entry,
            init_p2m,
        })
    }

    fn load(&self, image_info: &BootImageInfo, dst: &mut [u8]) -> Result<(), XenClientError> {
        let elf = ElfBytes::<AnyEndian>::minimal_parse(self.data.as_slice())?;
        let segments = elf.segments().ok_or(XenClientError::new(
            "Unable to parse kernel image: segments not found.",
        ))?;

        debug!(
            "ElfImageLoader load dst={:#x} segments={}",
            dst.as_ptr() as u64,
            segments.len()
        );
        for header in segments {
            let paddr = header.p_paddr;
            let filesz = header.p_filesz;
            let memsz = header.p_memsz;
            let base_offset = paddr - image_info.virt_kstart;
            let data = elf.segment_data(&header)?;
            let segment_dst = &mut dst[base_offset as usize..];
            let copy_slice = &data[0..filesz as usize];
            debug!(
                "ElfImageLoader load copy hdr={:?} dst={:#x} len={}",
                header,
                copy_slice.as_ptr() as u64,
                copy_slice.len()
            );
            copy(segment_dst, copy_slice);
            if memsz - filesz > 0 {
                let remaining = &mut segment_dst[filesz as usize..(memsz - filesz) as usize];
                debug!(
                    "ElfImageLoader load fill_zero hdr={:?} dst={:#x} len={}",
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
