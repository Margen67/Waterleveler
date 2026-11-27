// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67

use std::ffi::{c_int, c_void, CStr, CString};

/// The 64-bit Linux server's signatures. Always compiled (the tests check them against a
/// real `server_srv.so`), used only by the Linux build.
#[cfg(any(feature = "server", feature = "client"))]
#[allow(dead_code)]
pub(crate) mod linux_sigs {
    /// `client.so`: `C_BaseEntity::UpdateWaterState` (the `enginetrace` variable load).
    pub(crate) const CLIENT_TRACE_SIG: &str =
        "4C 8D 2D ?? ?? ?? ?? 31 D2 4C 89 E6 49 8B 7D 00 48 8B 07 FF 10 A9 30 40 00 00";
    pub(crate) const SIG: &str =
        "55 48 89 E5 41 56 41 55 41 54 41 89 F4 53 44 0F B6 AF ?? ?? 00 00 \
                       48 89 FB 41 38 F5 74 17 48 8B 07 48 8D B7 ?? ?? 00 00 FF 90 ?? ?? 00 00 \
                       44 88 A3 ?? ?? 00 00";
    pub(crate) const UPDATE_WATER_STATE_SIG: &str = "\
    55 48 89 E5 41 55 41 54 4C 8D 65 C0 53 48 8D 75 D0 4C 89 E2 48 89 FB 48 83 EC 28 \
    48 8B 05 ?? ?? ?? ?? 48 81 C7 ?? ?? 00 00 C7 45 C8 00 00 00 00 48 C7 45 C0 00 00 00 00 \
    C7 45 D8 00 00 00 00 48 89 45 D0 E8 ?? ?? ?? ?? 31 F6 48 89 DF E8 ?? ?? ?? ?? 31 F6 \
    48 89 DF E8 ?? ?? ?? ?? 4C 8D 2D ?? ?? ?? ?? 31 D2 4C 89 E6 49 8B 7D 00 48 8B 07 FF 10 \
    A9 30 40 00 00";
    pub(crate) const FLUID_END_TOUCH_SIG: &str =
        "55 48 85 F6 48 89 E5 41 56 49 89 D6 41 55 41 54 53 8B 87 F0 01 00 00 \
    48 89 FB 8D 50 01 89 97 F0 01 00 00 74 ?? 4D 85 F6 74 ?? 48 8B 06 49 89 F4 48 89 F7 \
    FF 90 90 00 00 00 48 85 C0 49 89 C5 74 ?? 48 89 C6 48 89 DF E8 ?? ?? ?? ?? 0F 2F 05 \
    ?? ?? ?? ?? 73 ?? 49 8B 45 00 4C 89 F7 41 81 A5 ?? ?? 00 00 FF FF F7 FF 4C 8B A0 F0 \
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
    /// The server's inline `Ray_t` setup (`UTIL_TraceLine`-style): `m_IsRay`, `m_IsSwept`, and
    /// the `lea rax,[ray]; mov rsi,rax` that passes the ray to `TraceRay`.
    pub(crate) const RAY_INIT_SIG: &str =
        "C6 85 ?? ?? ?? ?? 01 F3 0F 59 D2 F3 0F 11 8D ?? ?? ?? ?? F3 0F 59 C9 F3 0F 58 C2 \
    F3 0F 58 C1 66 0F EF C9 0F 2E C1 0F 9A C0 0F 45 C2 31 C9 31 D2 88 85 ?? ?? ?? ?? E8 \
    ?? ?? ?? ?? 4D 89 E8 48 89 D9 BA 0B 40 01 02 48 8D 05 ?? ?? ?? ?? 48 8B 38 \
    48 8D 85 ?? ?? ?? ?? 48 89 C6";
    /// The same in `client.so` (two sites, which must agree).
    pub(crate) const CLIENT_RAY_INIT_SIG: &str =
        "C6 85 ?? ?? ?? ?? 01 F3 0F 58 C4 F3 0F 58 C1 66 0F EF C9 0F 2E C1 0F 9A C0 0F 45 C2 \
    31 C9 31 D2 88 85 ?? ?? ?? ?? E8 ?? ?? ?? ?? 48 8D 05 ?? ?? ?? ?? 48 89 D9 BA 0B 40 ?? ?? \
    4C 8D 85 ?? ?? ?? ?? 4C 89 85 ?? ?? ?? ?? 48 8B 38 48 8D 85 ?? ?? ?? ?? 48 89 C6";
    pub(crate) const EXPLOSION_TARGETS: [&[&str]; 3] = [
        &[
            "55 48 89 E5 41 56 41 55 41 54 53 48 89 FB 48 81 EC 90 00 00 00 80 BF ?? 02 00 00 00 \
         0F 84 ?? ?? ?? ?? 48 8B 07 66 0F EF C0 48 C7 45 80 00 00 00 00",
        ],
        &[
            "55 48 89 E5 41 54 53 48 89 FB 48 81 EC A0 00 00 00 80 BF ?? 02 00 00 01 \
         0F 86 ?? ?? ?? ?? 48 8B 07 66 0F EF C0 48 C7 45 90 00 00 00 00",
        ],
        &[
            "55 48 89 E5 41 57 41 56 41 55 41 54 53 48 89 FB 48 81 EC A8 00 00 00 48 8B 3D \
         ?? ?? ?? ?? 48 8B 47 78 48 85 C0 74 ?? 80 38 00 0F 85 ?? ?? ?? ??",
        ],
    ];
}

// ---- Linux glue (thin on purpose) -------------------------------------

pub(crate) mod lin {
    #[allow(clippy::wildcard_imports)]
    use super::*;

    /// Load address and full path of the loaded library whose path ends with one of
    /// `suffixes` (for example `/server_srv.so`).
    unsafe fn loaded_module(suffixes: &[&[u8]]) -> Option<(usize, CString)> {
        unsafe {
            struct Search<'a> {
                suffixes: &'a [&'a [u8]],
                found: Option<(usize, CString)>,
            }
            unsafe extern "C" fn find(
                info: *mut libc::dl_phdr_info,
                _size: usize,
                data: *mut c_void,
            ) -> c_int {
                unsafe {
                    let search = &mut *data.cast::<Search<'_>>();
                    let name = (*info).dlpi_name;
                    if name.is_null() {
                        return 0;
                    }
                    let name = CStr::from_ptr(name);
                    if search.suffixes.iter().any(|s| name.to_bytes().ends_with(s)) {
                        search.found = Some(((*info).dlpi_addr as usize, name.to_owned()));
                        return 1;
                    }
                    0
                }
            }
            let mut search = Search {
                suffixes,
                found: None,
            };
            libc::dl_iterate_phdr(Some(find), (&raw mut search).cast::<c_void>());
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
            use libloading::os::unix::{Library, RTLD_LAZY};
            use std::ffi::c_char;
            let Some((_, path)) = loaded_module(&[b"/client.so"]) else {
                return 0;
            };
            // The library is already loaded, so this only takes a reference that is dropped
            // again below.
            let Ok(lib) =
                Library::open(Some(path.as_c_str().to_string_lossy().as_ref()), RTLD_LAZY)
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

    /// True when this library was loaded as the client module (file name gmcl_*).
    #[cfg(feature = "client")]
    pub(crate) unsafe fn is_client_module() -> bool {
        unsafe {
            let mut info: libc::Dl_info = std::mem::zeroed();
            if libc::dladdr(is_client_module as *const c_void, &raw mut info) == 0
                || info.dli_fname.is_null()
            {
                return false;
            }
            let path = CStr::from_ptr(info.dli_fname).to_bytes();
            let name = path.rsplit(|&c| c == b'/').next().unwrap_or(path);
            name.starts_with(b"gmcl_")
        }
    }
}

pub(crate) use lin as os;
