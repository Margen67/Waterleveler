// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67

use crate::*;
use std::ffi::{c_char, c_int, c_void, CStr, CString};

/// glibc's `dl_iterate_phdr` and the start of its `struct dl_phdr_info` (all that is read).
pub(crate) mod loaded_objects {
    use std::ffi::{c_char, c_int, c_void};

    #[repr(C)]
    pub(crate) struct DlPhdrInfo {
        pub(crate) addr: usize,
        pub(crate) name: *const c_char,
    }

    extern "C" {
        pub(crate) fn dl_iterate_phdr(
            callback: unsafe extern "C" fn(*const DlPhdrInfo, usize, *mut c_void) -> c_int,
            data: *mut c_void,
        ) -> c_int;
    }
}

pub(crate) unsafe fn find_symbol(name: &CStr) -> *mut c_void {
    unsafe {
        #[link(name = "dl")]
        extern "C" {
            fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
            fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
        }
        use loaded_objects::{dl_iterate_phdr, DlPhdrInfo};
        // lua_shared is already loaded by the game; RTLD_NOLOAD never loads anything new.
        const CANDIDATES: &[&CStr] = &[
            c"lua_shared.so",
            c"lua_shared_srv.so",
            c"bin/linux64/lua_shared.so",
            c"bin/linux64/lua_shared_srv.so",
            c"garrysmod/bin/linux64/lua_shared.so",
            c"garrysmod/bin/lua_shared_srv.so",
            c"garrysmod/bin/lua_shared.so",
            c"bin/linux32/lua_shared.so",
        ];
        const RTLD_LAZY: c_int = 0x1;
        const RTLD_NOLOAD: c_int = 0x4;

        // The game loads lua_shared by path, and dlopen only matches a bare name against a
        // loaded library's soname, so look the loaded library up by its full path first.
        unsafe extern "C" fn collect(
            info: *const DlPhdrInfo,
            _size: usize,
            data: *mut c_void,
        ) -> c_int {
            unsafe {
                let name = (*info).name;
                if !name.is_null() {
                    let name = CStr::from_ptr(name);
                    if name.to_bytes().windows(10).any(|w| w == b"lua_shared") {
                        (*data.cast::<Vec<CString>>()).push(name.to_owned());
                    }
                }
                0
            }
        }
        let mut loaded: Vec<CString> = Vec::new();
        dl_iterate_phdr(collect, (&raw mut loaded).cast::<c_void>());
        for path in &loaded {
            let handle = dlopen(path.as_ptr(), RTLD_LAZY | RTLD_NOLOAD);
            if !handle.is_null() {
                let sym = dlsym(handle, name.as_ptr());
                if !sym.is_null() {
                    return sym;
                }
            }
        }

        for lib in CANDIDATES {
            let handle = dlopen(lib.as_ptr(), RTLD_LAZY | RTLD_NOLOAD);
            if !handle.is_null() {
                let sym = dlsym(handle, name.as_ptr());
                if !sym.is_null() {
                    return sym;
                }
            }
        }
        // Fall back to the global scope (RTLD_DEFAULT == NULL on Linux).
        dlsym(std::ptr::null_mut(), name.as_ptr())
    }
}

/// The 64-bit Linux server's signatures. Always compiled (the tests check them against a
/// real `server_srv.so`), used only by the Linux build.
#[cfg(any(feature = "server", feature = "client"))]
#[allow(dead_code)]
pub(crate) mod linux_sigs {
    /// `client.so`: `C_BaseEntity::UpdateWaterState` (the `enginetrace` variable load).
    pub(crate) const CLIENT_TRACE_SIG: &str =
        "4C 8D 2D ?? ?? ?? ?? 31 D2 4C 89 E6 49 8B 7D 00 48 8B 07 FF 10 A9 30 40 00 00";
    pub(crate) const SIG: &str =
        "55 48 89 E5 41 56 41 55 41 54 41 89 F4 53 44 0F B6 AF D3 02 00 00 \
                       48 89 FB 41 38 F5 74 17 48 8B 07 48 8D B7 D3 02 00 00 FF 90 ?? ?? 00 00 \
                       44 88 A3 D3 02 00 00";
    pub(crate) const UPDATE_WATER_STATE_SIG: &str = "\
    55 48 89 E5 41 55 41 54 4C 8D 65 C0 53 48 8D 75 D0 4C 89 E2 48 89 FB 48 83 EC 28 \
    48 8B 05 ?? ?? ?? ?? 48 81 C7 10 02 00 00 C7 45 C8 00 00 00 00 48 C7 45 C0 00 00 00 00 \
    C7 45 D8 00 00 00 00 48 89 45 D0 E8 ?? ?? ?? ?? 31 F6 48 89 DF E8 ?? ?? ?? ?? 31 F6 \
    48 89 DF E8 ?? ?? ?? ?? 4C 8D 2D ?? ?? ?? ?? 31 D2 4C 89 E6 49 8B 7D 00 48 8B 07 FF 10 \
    A9 30 40 00 00";
    pub(crate) const FLUID_END_TOUCH_SIG: &str =
        "55 48 85 F6 48 89 E5 41 56 49 89 D6 41 55 41 54 53 8B 87 F0 01 00 00 \
    48 89 FB 8D 50 01 89 97 F0 01 00 00 74 ?? 4D 85 F6 74 ?? 48 8B 06 49 89 F4 48 89 F7 \
    FF 90 90 00 00 00 48 85 C0 49 89 C5 74 ?? 48 89 C6 48 89 DF E8 ?? ?? ?? ?? 0F 2F 05 \
    ?? ?? ?? ?? 73 ?? 49 8B 45 00 4C 89 F7 41 81 A5 B8 01 00 00 FF FF F7 FF 4C 8B A0 F0 \
    02 00 00 49 8B 06 FF 50 38 48 63 D0 BE 01 00 00 00 4C 89 EF 41 FF D4 8B 83 F0 01 00 00";
    pub(crate) const BUBBLE_TRAIL_SIG: &str =
        "55 48 89 E5 41 57 41 56 41 55 41 89 D5 41 54 49 89 F4 53 48 89 FB 48 83 EC 68 \
    F3 0F 10 47 08 F3 0F 10 0D ?? ?? ?? ?? F3 0F 58 C8 E8 ?? ?? ?? ?? F3 0F 10 0D ?? ?? ?? ?? \
    4C 89 E7 0F 28 D0 F3 41 0F 10 44 24 08 F3 0F 5C 53 08";
    pub(crate) const DISPATCH_EFFECT_SIG: &str =
        "55 48 89 E5 41 56 41 55 4C 8D 35 ?? ?? ?? ?? 49 89 FD 41 54 4C 8D 65 A0 53 4C 89 E7 \
    48 89 F3 48 83 EC 40 E8 ?? ?? ?? ?? 48 89 DE 4C 89 E7";
    pub(crate) const TE_CREATE_SIG: &str =
        "55 48 89 E5 41 57 41 56 4C 8D 35 ?? ?? ?? ?? 41 55 41 54 4C 8D 25 ?? ?? ?? ?? 53 \
    48 89 FB 48 83 EC 28 48 8D 05 ?? ?? ?? ?? 44 8B 05 ?? ?? ?? ?? 4C 8B 28";
    pub(crate) const PHYSENV_SIG: &str =
        "48 8B 3D ?? ?? ?? ?? 48 8D 95 ?? ?? ?? ?? 4C 89 E6 48 8B 07 FF 50 60";
    pub(crate) const ENTITY_ARRAY_SIG: &str =
        "48 8D 15 ?? ?? ?? ?? 8B 00 48 8B 12 83 F8 FF 74 ?? 89 C1 C1 E8 0E 81 E1 FF 3F 00 00 \
    48 C1 E1 05";
    pub(crate) const EXPLOSION_TARGETS: [&[(&str, usize)]; 3] = [
        &[(
            "55 48 89 E5 41 56 41 55 41 54 53 48 89 FB 48 81 EC 90 00 00 00 80 BF D3 02 00 00 00 \
         0F 84 ?? ?? ?? ?? 48 8B 07 66 0F EF C0 48 C7 45 80 00 00 00 00",
            14,
        )],
        &[(
            "55 48 89 E5 41 54 53 48 89 FB 48 81 EC A0 00 00 00 80 BF D3 02 00 00 01 \
         0F 86 ?? ?? ?? ?? 48 8B 07 66 0F EF C0 48 C7 45 90 00 00 00 00",
            17,
        )],
        &[(
            "55 48 89 E5 41 57 41 56 41 55 41 54 53 48 89 FB 48 81 EC A8 00 00 00 48 8B 3D \
         ?? ?? ?? ?? 48 8B 47 78 48 85 C0 74 ?? 80 38 00 0F 85 ?? ?? ?? ??",
            16,
        )],
    ];
}

/// The tests scan mapped Windows images on a Linux host too.
#[cfg(all(any(feature = "server", feature = "client"), not(windows)))]
pub(crate) unsafe fn executable_regions(
    base: *const u8,
) -> Result<Vec<(usize, usize)>, &'static str> {
    unsafe {
        #[cfg(test)]
        if std::slice::from_raw_parts(base, 2) == b"MZ" {
            return pe_regions(base);
        }
        elf_regions(base)
    }
}

#[cfg(all(any(feature = "server", feature = "client"), not(windows)))]
pub(crate) unsafe fn elf_regions(base: *const u8) -> Result<Vec<(usize, usize)>, &'static str> {
    unsafe {
        const PT_LOAD: u32 = 1;
        const PF_X: u32 = 1;
        let rd64 = |off: usize| base.add(off).cast::<u64>().read_unaligned();
        let rd32 = |off: usize| base.add(off).cast::<u32>().read_unaligned();
        let rd16 = |off: usize| base.add(off).cast::<u16>().read_unaligned();
        if std::slice::from_raw_parts(base, 4) != b"\x7fELF" {
            return Err("module has no ELF header");
        }
        if *base.add(4) != 2 || rd16(18) != 62 {
            return Err("module is not a 64-bit x86-64 ELF image");
        }
        let phoff = rd64(0x20) as usize;
        let phentsize = rd16(0x36) as usize;
        let phnum = rd16(0x38) as usize;
        let mut regions = Vec::new();
        for i in 0..phnum {
            let ph = phoff + i * phentsize;
            if rd32(ph) == PT_LOAD && rd32(ph + 4) & PF_X != 0 {
                // p_vaddr, p_memsz
                regions.push((rd64(ph + 0x10) as usize, rd64(ph + 0x28) as usize));
            }
        }
        Ok(regions)
    }
}

// ---- Windows glue (thin on purpose) ----------------------------------

// ---- Linux glue (thin on purpose) -------------------------------------

pub(crate) mod lin {
    #[allow(clippy::wildcard_imports)]
    use super::*;

    const PROT_READ: c_int = 1;
    const PROT_WRITE: c_int = 2;
    const PROT_EXEC: c_int = 4;
    const MAP_PRIVATE: c_int = 2;
    const MAP_ANONYMOUS: c_int = 0x20;
    const PAGE: usize = 4096;

    extern "C" {
        #[cfg(feature = "server")]
        fn mmap(
            addr: *mut c_void,
            len: usize,
            prot: c_int,
            flags: c_int,
            fd: c_int,
            off: i64,
        ) -> *mut c_void;
        fn mprotect(addr: *mut c_void, len: usize, prot: c_int) -> c_int;
    }

    #[cfg(feature = "server")]
    unsafe fn alloc_exec(size: usize) -> *mut u8 {
        unsafe {
            let p = mmap(
                std::ptr::null_mut(),
                size,
                PROT_READ | PROT_WRITE | PROT_EXEC,
                MAP_PRIVATE | MAP_ANONYMOUS,
                -1,
                0,
            );
            if p as isize == -1 {
                std::ptr::null_mut()
            } else {
                p.cast::<u8>()
            }
        }
    }

    /// Protection bits (`PROT_*`) of the mapping that holds `addr`.
    fn protection_of(addr: usize) -> Option<c_int> {
        let maps = std::fs::read_to_string("/proc/self/maps").ok()?;
        for line in maps.lines() {
            let mut it = line.split_whitespace();
            let (range, perms) = (it.next()?, it.next()?);
            let (lo, hi) = range.split_once('-')?;
            let (lo, hi) = (
                usize::from_str_radix(lo, 16).ok()?,
                usize::from_str_radix(hi, 16).ok()?,
            );
            if (lo..hi).contains(&addr) {
                let p = perms.as_bytes();
                return Some(
                    if p.first() == Some(&b'r') {
                        PROT_READ
                    } else {
                        0
                    } | if p.get(1) == Some(&b'w') {
                        PROT_WRITE
                    } else {
                        0
                    } | if p.get(2) == Some(&b'x') {
                        PROT_EXEC
                    } else {
                        0
                    },
                );
            }
        }
        None
    }

    fn page_span(p: *mut u8, n: usize) -> (*mut c_void, usize) {
        let start = p as usize & !(PAGE - 1);
        let end = (p as usize + n).div_ceil(PAGE) * PAGE;
        (start as *mut c_void, end - start)
    }

    unsafe fn make_writable(p: *mut u8, n: usize) -> Option<u32> {
        unsafe {
            let old = protection_of(p as usize)?;
            let (start, len) = page_span(p, n);
            (mprotect(start, len, old | PROT_WRITE) == 0).then_some(old as u32)
        }
    }
    unsafe fn restore_protect(p: *mut u8, n: usize, old: u32) {
        unsafe {
            let (start, len) = page_span(p, n);
            mprotect(start, len, old as c_int);
        }
    }
    #[cfg(feature = "server")]
    const unsafe fn flush(_p: *mut u8, _n: usize) {
        // x86 keeps instruction fetches coherent with stores
    }

    pub(crate) const PLATFORM: Platform = Platform {
        #[cfg(feature = "server")]
        alloc_exec,
        make_writable,
        restore_protect,
        #[cfg(feature = "server")]
        flush,
    };

    use crate::loaded_objects::{dl_iterate_phdr, DlPhdrInfo};

    /// Load address and full path of the loaded library whose path ends with one of
    /// `suffixes` (for example `/server_srv.so`).
    unsafe fn loaded_module(suffixes: &[&[u8]]) -> Option<(usize, CString)> {
        unsafe {
            struct Search<'a> {
                suffixes: &'a [&'a [u8]],
                found: Option<(usize, CString)>,
            }
            unsafe extern "C" fn find(
                info: *const DlPhdrInfo,
                _size: usize,
                data: *mut c_void,
            ) -> c_int {
                unsafe {
                    let search = &mut *data.cast::<Search<'_>>();
                    let name = (*info).name;
                    if name.is_null() {
                        return 0;
                    }
                    let name = CStr::from_ptr(name);
                    if search.suffixes.iter().any(|s| name.to_bytes().ends_with(s)) {
                        search.found = Some(((*info).addr, name.to_owned()));
                        return 1;
                    }
                    0
                }
            }
            let mut search = Search {
                suffixes,
                found: None,
            };
            dl_iterate_phdr(find, (&raw mut search).cast::<c_void>());
            search.found
        }
    }

    /// Load address of the 64-bit Linux server module (`server_srv.so`, or `server.so`).
    #[cfg(feature = "server")]
    pub(crate) unsafe fn server_base() -> *const u8 {
        unsafe {
            loaded_module(&[b"/server_srv.so", b"/server.so"])
                .map_or(std::ptr::null(), |(base, _)| base as *const u8)
        }
    }

    /// Load address of the 64-bit Linux client module (`client.so`).
    #[cfg(feature = "client")]
    pub(crate) unsafe fn client_base() -> *const u8 {
        unsafe {
            loaded_module(&[b"/client.so"]).map_or(std::ptr::null(), |(base, _)| base as *const u8)
        }
    }

    #[cfg(feature = "client")]
    /// An interface of client.so (`CreateInterface("VClientEntityList003")`).
    pub(crate) unsafe fn client_interface(name: &CStr) -> usize {
        unsafe {
            #[link(name = "dl")]
            extern "C" {
                fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
                fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
            }
            const RTLD_LAZY: c_int = 0x1;
            const RTLD_NOLOAD: c_int = 0x4;
            let Some((_, path)) = loaded_module(&[b"/client.so"]) else {
                return 0;
            };
            let handle = dlopen(path.as_ptr(), RTLD_LAZY | RTLD_NOLOAD);
            if handle.is_null() {
                return 0;
            }
            let create = dlsym(handle, c"CreateInterface".as_ptr());
            if create.is_null() {
                return 0;
            }
            let create: engine_fn!((*const c_char, *mut c_int) -> usize) =
                std::mem::transmute(create);
            create(name.as_ptr(), std::ptr::null_mut())
        }
    }

    /// True when this library was loaded as the client module (file name gmcl_*).
    #[cfg(feature = "client")]
    pub(crate) unsafe fn is_client_module() -> bool {
        unsafe {
            #[repr(C)]
            struct DlInfo {
                fname: *const c_char,
                fbase: *mut c_void,
                sname: *const c_char,
                saddr: *mut c_void,
            }
            #[link(name = "dl")]
            extern "C" {
                fn dladdr(addr: *const c_void, info: *mut DlInfo) -> c_int;
            }
            let mut info = DlInfo {
                fname: std::ptr::null(),
                fbase: std::ptr::null_mut(),
                sname: std::ptr::null(),
                saddr: std::ptr::null_mut(),
            };
            if dladdr(is_client_module as *const c_void, &mut info) == 0 || info.fname.is_null() {
                return false;
            }
            let path = CStr::from_ptr(info.fname).to_bytes();
            let name = path.rsplit(|&c| c == b'/').next().unwrap_or(path);
            name.starts_with(b"gmcl_")
        }
    }
}

pub(crate) use lin as os;
