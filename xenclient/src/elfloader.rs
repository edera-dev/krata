use crate::boot::{BootImageInfo, BootImageLoader};
use crate::sys::{XEN_ELFNOTE_ENTRY, XEN_ELFNOTE_HV_START_LOW, XEN_ELFNOTE_VIRT_BASE};
use crate::XenClientError;
use elf::abi::{PF_R, PF_W, PF_X, PT_LOAD, SHT_NOTE};
use elf::endian::AnyEndian;
use elf::note::Note;
use elf::{ElfBytes, ParseError};
use flate2::bufread::GzDecoder;
use memchr::memmem::find_iter;
use std::collections::HashMap;
use std::io::{BufReader, Read};
use std::mem::size_of;
use xz2::bufread::XzDecoder;

impl From<ParseError> for XenClientError {
    fn from(value: ParseError) -> Self {
        XenClientError::new(value.to_string().as_str())
    }
}

pub struct ElfImageLoader {
    data: Vec<u8>,
}

fn xen_note_value_u64(endian: AnyEndian, notes: &HashMap<u64, Vec<u8>>, key: u64) -> Option<u64> {
    let value = notes.get(&key);
    value?;
    let value = value.unwrap();
    let bytes: Option<[u8; size_of::<u64>()]> = value.clone().try_into().ok();
    bytes?;

    Some(match endian {
        AnyEndian::Little => u64::from_le_bytes(bytes.unwrap()),
        AnyEndian::Big => u64::from_be_bytes(bytes.unwrap()),
    })
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
            match ElfImageLoader::load_xz(&file[start..]) {
                Ok(elf) => return Ok(elf),
                Err(error) => {
                    println!("{}", error);
                }
            }
        }

        Err(XenClientError::new(
            "Unable to parse kernel image: unknown compression type",
        ))
    }
}

impl BootImageLoader for ElfImageLoader {
    fn load(&self, dst: *mut u8) -> Result<BootImageInfo, XenClientError> {
        let elf = ElfBytes::<AnyEndian>::minimal_parse(self.data.as_slice())?;
        let headers = elf.section_headers().ok_or(XenClientError::new(
            "Unable to parse kernel image: section headers not found.",
        ))?;
        let mut linux_notes: HashMap<u64, Vec<u8>> = HashMap::new();
        let mut xen_notes: HashMap<u64, Vec<u8>> = HashMap::new();

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
                        xen_notes.insert(note.n_type, note.desc.to_vec());
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

        let virt_base = xen_note_value_u64(elf.ehdr.endianness, &xen_notes, XEN_ELFNOTE_VIRT_BASE)
            .ok_or(XenClientError::new(
                "Unable to find virt_base note in kernel.",
            ))?;
        let entry = xen_note_value_u64(elf.ehdr.endianness, &xen_notes, XEN_ELFNOTE_ENTRY)
            .ok_or(XenClientError::new("Unable to find entry note in kernel."))?;
        let hv_start_low =
            xen_note_value_u64(elf.ehdr.endianness, &xen_notes, XEN_ELFNOTE_HV_START_LOW).ok_or(
                XenClientError::new("Unable to find hv_start_low note in kernel."),
            )?;

        let mut start: u64 = u64::MAX;
        let mut end: u64 = 0;

        let segments = elf.segments().ok_or(XenClientError::new(
            "Unable to parse kernel image: segments not found.",
        ))?;
        for segment in segments {
            if (segment.p_type != PT_LOAD) || (segment.p_flags & (PF_R | PF_W | PF_X)) == 0 {
                continue;
            }
            let paddr = segment.p_paddr;
            let memsz = segment.p_memsz;
            if start > paddr {
                start = paddr;
            }

            if end < paddr + memsz {
                end = paddr + memsz;
            }
        }

        let base_dst_addr = dst as u64;
        for header in segments {
            let paddr = header.p_paddr;
            let filesz = header.p_filesz;
            let memsz = header.p_memsz;
            let dest = base_dst_addr + paddr - start;
            let data = elf.segment_data(&header)?;

            unsafe {
                std::ptr::copy(data.as_ptr(), dest as *mut u8, filesz as usize);
                std::ptr::write_bytes((dest + filesz) as *mut u8, 0, (memsz - filesz) as usize);
            }
        }

        let virt_base = if virt_base == u64::MAX { 0 } else { virt_base };

        let virt_kstart = start + virt_base;
        let virt_kend = end + virt_base;

        Ok(BootImageInfo {
            virt_kstart,
            virt_kend,
            entry,
            hv_start_low,
        })
    }
}
