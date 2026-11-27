// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67

#[cfg(all(windows, feature = "client"))]
use crate::*;
#[cfg(all(windows, feature = "client"))]
use std::ffi::{c_char, c_int};

#[cfg(windows)]
pub(crate) mod win {
    /// Windows modules address their globals directly (`lea`/`mov` of the variable itself).
    pub(crate) const fn resolve_global(address: usize) -> usize {
        address
    }

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
