// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67

#[cfg(all(windows, feature = "client"))]
use crate::*;
#[cfg(all(windows, feature = "client"))]
use std::ffi::{c_char, c_int};

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
    #[cfg(feature = "client")]
    #[allow(clippy::wildcard_imports)]
    use super::*;
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleA;

    #[cfg(feature = "server")]
    pub(crate) unsafe fn server_base() -> *const u8 {
        unsafe { GetModuleHandleA(c"server.dll".as_ptr().cast()) as *const u8 }
    }

    #[cfg(feature = "client")]
    pub(crate) unsafe fn client_base() -> *const u8 {
        unsafe { GetModuleHandleA(c"client.dll".as_ptr().cast()) as *const u8 }
    }

    #[cfg(feature = "client")]
    /// An interface of client.dll (`CreateInterface("VClientEntityList003")`).
    pub(crate) unsafe fn client_interface(name: &CStr) -> usize {
        unsafe {
            // Takes a reference on the already loaded module, which `Drop` gives back.
            let Ok(lib) = libloading::os::windows::Library::open_already_loaded("client.dll")
            else {
                return 0;
            };
            let Ok(create) =
                lib.get::<engine_fn!((*const c_char, *mut c_int) -> usize)>(b"CreateInterface\0")
            else {
                return 0;
            };
            create(name.as_ptr(), std::ptr::null_mut())
        }
    }
}

#[cfg(windows)]
pub(crate) use win as os;
