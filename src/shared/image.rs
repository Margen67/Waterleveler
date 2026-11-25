// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67

use object::elf::{FileHeader64, EM_X86_64, PF_X, PT_LOAD};
use object::pe::{ImageDosHeader, ImageNtHeaders64, IMAGE_SCN_MEM_EXECUTE};
use object::read::elf::{FileHeader, ProgramHeader};
use object::read::pe::ImageNtHeaders;
use object::{Endianness, FileKind, LittleEndian};

/// The headers of a loaded image are read from its first page, which is always mapped.
const HEADER_BYTES: usize = 0x1000;

/// The executable parts of a loaded module as (offset from `base`, size): the executable
/// sections of a PE image, the executable segments of an ELF one. Both are 64-bit x86-64
/// only. The tests map Windows images on a Linux host too, so either format is accepted.
#[cfg(any(feature = "server", feature = "client"))]
pub(crate) unsafe fn executable_regions(
    base: *const u8,
) -> Result<Vec<(usize, usize)>, &'static str> {
    // SAFETY: the caller passes the base of a mapped module, whose first page is readable.
    let head = unsafe { std::slice::from_raw_parts(base, HEADER_BYTES) };
    match FileKind::parse(head) {
        Ok(FileKind::Pe64) => pe_regions(head),
        Ok(FileKind::Elf64) => elf_regions(head),
        Ok(_) => Err("module is not a 64-bit PE or ELF image"),
        Err(_) => Err("module is not a PE or ELF image"),
    }
}

fn pe_regions(head: &[u8]) -> Result<Vec<(usize, usize)>, &'static str> {
    const BAD: &str = "module has no valid PE header";
    let dos = ImageDosHeader::parse(head).map_err(|_| BAD)?;
    let mut offset = u64::from(dos.nt_headers_offset());
    let (nt, _) = ImageNtHeaders64::parse(head, &mut offset).map_err(|_| BAD)?;
    let sections = nt.sections(head, offset).map_err(|_| BAD)?;
    Ok(sections
        .iter()
        .filter(|s| s.characteristics.get(LittleEndian).0 & IMAGE_SCN_MEM_EXECUTE.0 != 0)
        .map(|s| {
            (
                s.virtual_address.get(LittleEndian) as usize,
                s.virtual_size.get(LittleEndian) as usize,
            )
        })
        .collect())
}

fn elf_regions(head: &[u8]) -> Result<Vec<(usize, usize)>, &'static str> {
    const BAD: &str = "module has no valid ELF header";
    let header = FileHeader64::<Endianness>::parse(head).map_err(|_| BAD)?;
    let endian = header.endian().map_err(|_| BAD)?;
    if header.e_machine(endian) != EM_X86_64 {
        return Err("module is not a 64-bit x86-64 ELF image");
    }
    let segments = header.program_headers(endian, head).map_err(|_| BAD)?;
    Ok(segments
        .iter()
        .filter(|p| p.p_type(endian) == PT_LOAD && p.p_flags(endian).0 & PF_X.0 != 0)
        .map(|p| (p.p_vaddr(endian) as usize, p.p_memsz(endian) as usize))
        .collect())
}
