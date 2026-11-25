// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67

#[cfg(windows)]
use crate::*;
#[cfg(windows)]
use std::ffi::{c_char, c_void};

/// The executable parts of a loaded module as (offset from `base`, size): the
/// executable sections of a PE image, the executable segments of an ELF one.
#[cfg(all(windows, any(feature = "server", feature = "client")))]
pub(crate) unsafe fn executable_regions(
    base: *const u8,
) -> Result<Vec<(usize, usize)>, &'static str> {
    unsafe { pe_regions(base) }
}

#[cfg(all(any(windows, test), any(feature = "server", feature = "client")))]
pub(crate) unsafe fn pe_regions(base: *const u8) -> Result<Vec<(usize, usize)>, &'static str> {
    unsafe {
        let rd32 = |off: usize| base.add(off).cast::<u32>().read_unaligned();
        let rd16 = |off: usize| base.add(off).cast::<u16>().read_unaligned();
        if rd16(0) != 0x5A4D {
            return Err("module has no MZ header");
        }
        let nt = rd32(0x3C) as usize;
        if rd32(nt) != 0x0000_4550 {
            return Err("module has no PE header");
        }
        if rd16(nt + 24) != 0x20B {
            return Err("module is not a 64-bit PE image");
        }
        let nsec = rd16(nt + 6) as usize;
        let optsz = rd16(nt + 20) as usize;
        let sections = nt + 24 + optsz;
        let mut regions = Vec::new();
        for i in 0..nsec {
            let s = sections + i * 40;
            if rd32(s + 36) & 0x2000_0000 != 0 {
                regions.push((rd32(s + 12) as usize, rd32(s + 8) as usize));
            }
        }
        Ok(regions)
    }
}

#[cfg(windows)]
pub(crate) mod win {
    #[allow(clippy::wildcard_imports)]
    // the Windows code needs more names than the Linux server uses
    use super::*;

    #[link(name = "kernel32")]
    extern "system" {
        fn GetModuleHandleA(name: *const c_char) -> *mut c_void;
        #[cfg(feature = "server")]
        fn VirtualAlloc(addr: *mut c_void, size: usize, ty: u32, prot: u32) -> *mut c_void;
        fn VirtualProtect(addr: *mut c_void, size: usize, new: u32, old: *mut u32) -> i32;
        #[cfg(feature = "server")]
        fn FlushInstructionCache(process: *mut c_void, addr: *const c_void, size: usize) -> i32;
        #[cfg(feature = "server")]
        fn GetCurrentProcess() -> *mut c_void;
    }
    #[cfg(feature = "server")]
    const MEM_COMMIT_RESERVE: u32 = 0x3000;
    const PAGE_EXECUTE_READWRITE: u32 = 0x40;

    #[cfg(feature = "server")]
    unsafe fn alloc_exec(size: usize) -> *mut u8 {
        unsafe {
            VirtualAlloc(
                std::ptr::null_mut(),
                size,
                MEM_COMMIT_RESERVE,
                PAGE_EXECUTE_READWRITE,
            )
            .cast::<u8>()
        }
    }
    unsafe fn make_writable(p: *mut u8, n: usize) -> Option<u32> {
        unsafe {
            let mut old = 0u32;
            if VirtualProtect(p.cast::<c_void>(), n, PAGE_EXECUTE_READWRITE, &raw mut old) != 0 {
                Some(old)
            } else {
                None
            }
        }
    }
    unsafe fn restore_protect(p: *mut u8, n: usize, old: u32) {
        unsafe {
            let mut tmp = 0u32;
            VirtualProtect(p.cast::<c_void>(), n, old, &raw mut tmp);
        }
    }
    #[cfg(feature = "server")]
    unsafe fn flush(p: *mut u8, n: usize) {
        unsafe {
            FlushInstructionCache(GetCurrentProcess(), p as *const c_void, n);
        }
    }

    pub(crate) const PLATFORM: Platform = Platform {
        #[cfg(feature = "server")]
        alloc_exec,
        make_writable,
        restore_protect,
        #[cfg(feature = "server")]
        flush,
    };

    #[cfg(feature = "server")]
    pub(crate) unsafe fn server_base() -> *const u8 {
        unsafe { GetModuleHandleA(c"server.dll".as_ptr()) as *const u8 }
    }

    #[cfg(feature = "client")]
    pub(crate) unsafe fn client_base() -> *const u8 {
        unsafe { GetModuleHandleA(c"client.dll".as_ptr()) as *const u8 }
    }

    #[cfg(feature = "client")]
    /// An interface of client.dll (`CreateInterface("VClientEntityList003")`).
    pub(crate) unsafe fn client_interface(name: &CStr) -> usize {
        unsafe {
            #[link(name = "kernel32")]
            extern "system" {
                fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *mut c_void;
            }
            let module = client_base() as *mut c_void;
            if module.is_null() {
                return 0;
            }
            let create = GetProcAddress(module, c"CreateInterface".as_ptr());
            if create.is_null() {
                return 0;
            }
            let create: engine_fn!((*const c_char, *mut c_int) -> usize) =
                std::mem::transmute(create);
            create(name.as_ptr(), std::ptr::null_mut())
        }
    }
}

#[cfg(windows)]
pub(crate) use win as os;
