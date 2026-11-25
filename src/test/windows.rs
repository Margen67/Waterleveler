// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67

use super::*;

extern "C" {
    pub(crate) fn mmap(
        addr: *mut c_void,
        len: usize,
        prot: c_int,
        flags: c_int,
        fd: c_int,
        off: i64,
    ) -> *mut c_void;
}

pub(crate) fn server_dll() -> Option<Vec<u8>> {
    let path = std::env::var("SERVER_DLL_X64").ok()?;
    std::fs::read(path).ok()
}

pub(crate) fn client_dll() -> Option<Vec<u8>> {
    let path = std::env::var("CLIENT_DLL_X64").ok()?;
    std::fs::read(path).ok()
}

pub(crate) struct Pe {
    pub(crate) size_of_image: usize,
    pub(crate) size_of_headers: usize,
    /// (name, rva, vsize, raw offset, raw size, characteristics)
    pub(crate) sections: Vec<(String, usize, usize, usize, usize, u32)>,
}

pub(crate) fn parse_pe(d: &[u8]) -> Pe {
    let u32at = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap()) as usize;
    let u16at = |o: usize| u16::from_le_bytes(d[o..o + 2].try_into().unwrap()) as usize;
    let nt = u32at(0x3C);
    assert_eq!(&d[nt..nt + 4], b"PE\0\0");
    assert_eq!(u16at(nt + 24), 0x20B, "expected a PE32+ image");
    let nsec = u16at(nt + 6);
    let optsz = u16at(nt + 20);
    let opt = nt + 24;
    let mut sections = Vec::new();
    for i in 0..nsec {
        let s = opt + optsz + i * 40;
        let name = String::from_utf8_lossy(&d[s..s + 8])
            .trim_end_matches('\0')
            .to_string();
        sections.push((
            name,
            u32at(s + 12),
            u32at(s + 8),
            u32at(s + 20),
            u32at(s + 16),
            u32at(s + 36) as u32,
        ));
    }
    Pe {
        size_of_image: u32at(opt + 56),
        size_of_headers: u32at(opt + 60),
        sections,
    }
}

/// Lays the file out the way the Windows loader would (headers + sections).
pub(crate) fn map_image(d: &[u8], pe: &Pe) -> Vec<u8> {
    let mut img = vec![0u8; pe.size_of_image];
    img[..pe.size_of_headers].copy_from_slice(&d[..pe.size_of_headers]);
    for (_, rva, _, raw, rawsz, _) in &pe.sections {
        img[*rva..*rva + *rawsz].copy_from_slice(&d[*raw..*raw + *rawsz]);
    }
    img
}

pub(crate) unsafe fn rwx(len: usize) -> *mut u8 {
    unsafe {
        // PROT_READ|WRITE|EXEC, MAP_PRIVATE|MAP_ANONYMOUS
        let p = mmap(std::ptr::null_mut(), len, 7, 0x22, -1, 0);
        assert_ne!(p as isize, -1, "mmap failed");
        p.cast::<u8>()
    }
}

pub(crate) unsafe fn t_alloc(n: usize) -> *mut u8 {
    unsafe { rwx(n.max(4096)) }
}

// Same signature as the Windows implementation it stands in for.
#[allow(clippy::unnecessary_wraps)]
pub(crate) const unsafe fn t_writable(_: *mut u8, _: usize) -> Option<u32> {
    Some(0)
}

pub(crate) const unsafe fn t_restore(_: *mut u8, _: usize, _: u32) {}

pub(crate) const unsafe fn t_flush(_: *mut u8, _: usize) {}

pub(crate) const TEST_PLATFORM: Platform = Platform {
    alloc_exec: t_alloc,
    make_writable: t_writable,
    restore_protect: t_restore,
    flush: t_flush,
};
