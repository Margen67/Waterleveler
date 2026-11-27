// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67

use std::ffi::{c_int, c_void, CStr, CString};

/// The 64-bit Linux server's and client's signatures. Always compiled (the tests check them
/// against the real `server.so` and `client.so`), used only by the Linux build.
#[cfg(any(feature = "server", feature = "client"))]
#[allow(dead_code)]
pub(crate) mod linux_sigs {
    /// `client.so`: `C_BaseEntity::UpdateWaterState` (the `enginetrace` variable load).
    pub(crate) const CLIENT_TRACE_SIG: &str = concat!(
        "4C 8D 2D ?? ?? ?? ?? 31 D2 4C 89 E6 49 8B 7D 00 48 8B 07 FF 10 A9 30 40 00 00",
        " | ",
        "4C 8B 2D ?? ?? ?? ?? 31 D2 4C 89 E6 49 8B 7D 00 48 8B 07 FF 10 A9 30 40 00 00",
    );
    pub(crate) const SIG: &str =
        concat!(
        "55 48 89 E5 41 56 41 55 41 54 41 89 F4 53 44 0F B6 AF ?? ?? 00 00 \
                       48 89 FB 41 38 F5 74 17 48 8B 07 48 8D B7 ?? ?? 00 00 FF 90 ?? ?? 00 00 \
                       44 88 A3 ?? ?? 00 00",
        " | ",
        "55 48 89 E5 41 56 41 55 41 54 41 89 F4 53 48 89 FB 0F B6 87 ?? ?? 00 00 40 38 F0 44 0F B6 E8 74 ?? 48 8B 07 48 8D B7 ?? ?? 00 00 FF 90 ?? ?? 00 00 44 88 A3 ?? ?? 00 00",
    );
    pub(crate) const UPDATE_WATER_STATE_SIG: &str = concat!(
        "\
    55 48 89 E5 41 55 41 54 4C 8D 65 C0 53 48 8D 75 D0 4C 89 E2 48 89 FB 48 83 EC 28 \
    48 8B 05 ?? ?? ?? ?? 48 81 C7 ?? ?? 00 00 C7 45 C8 00 00 00 00 48 C7 45 C0 00 00 00 00 \
    C7 45 D8 00 00 00 00 48 89 45 D0 E8 ?? ?? ?? ?? 31 F6 48 89 DF E8 ?? ?? ?? ?? 31 F6 \
    48 89 DF E8 ?? ?? ?? ?? 4C 8D 2D ?? ?? ?? ?? 31 D2 4C 89 E6 49 8B 7D 00 48 8B 07 FF 10 \
    A9 30 40 00 00",
        " | ",
        "55 0F 57 C0 48 89 E5 41 55 41 54 4C 8D 65 C0 53 4C 89 E2 48 89 FB 48 8D 75 D0 48 83 EC 38 F3 0F 11 45 C8 48 8D BF ?? ?? 00 00 F3 0F 11 45 C4 F3 0F 11 45 C0 F3 0F 11 45 D8 F3 0F 11 45 BC C7 45 D0 00 00 00 3F C7 45 D4 00 00 00 3F E8 ?? ?? ?? ?? 31 F6 48 89 DF E8 ?? ?? ?? ?? 31 F6 48 89 DF E8 ?? ?? ?? ?? 4C 8B 2D ?? ?? ?? ?? 31 D2 4C 89 E6 49 8B 7D 00 48 8B 07 FF 10 A9 30 40 00 00",
    );
    pub(crate) const FLUID_END_TOUCH_SIG: &str =
        concat!(
        "55 48 85 F6 48 89 E5 41 56 49 89 D6 41 55 41 54 53 8B 87 F0 01 00 00 \
    48 89 FB 8D 50 01 89 97 F0 01 00 00 74 ?? 4D 85 F6 74 ?? 48 8B 06 49 89 F4 48 89 F7 \
    FF 90 90 00 00 00 48 85 C0 49 89 C5 74 ?? 48 89 C6 48 89 DF E8 ?? ?? ?? ?? 0F 2F 05 \
    ?? ?? ?? ?? 73 ?? 49 8B 45 00 4C 89 F7 41 81 A5 ?? ?? 00 00 FF FF F7 FF 4C 8B A0 F0 \
    02 00 00 49 8B 06 FF 50 38 48 63 D0 BE 01 00 00 00 4C 89 EF 41 FF D4 8B 83 F0 01 00 00",
        " | ",
        "55 48 89 E5 41 56 41 55 41 54 49 89 F4 53 48 89 FB 8B 87 F0 01 00 00 83 C0 01 48 85 F6 89 87 F0 01 00 00 75 ?? 83 E8 01 89 83 F0 01 00 00 5B 41 5C 41 5D 41 5E 5D C3 66 0F 1F 84 00 00 00 00 00 48 85 D2 49 89 D5 74 ?? 48 8B 06 48 89 F7 FF 90 90 00 00 00 48 85 C0 49 89 C6 74 ?? 48 89 C6 48 89 DF E8 ?? ?? ?? ?? 0F 2F 05 ?? ?? ?? ?? 73 ?? 41 81 A6 ?? ?? 00 00 FF FF F7 FF 4C 89 EF 49 8B 06 4C 8B A0 F0 02 00 00 49 8B 45 00 FF 50 38 48 63 D0 BE 01 00 00 00 4C 89 F7",
    );
    pub(crate) const BUBBLE_TRAIL_SIG: &str =
        concat!(
        "55 48 89 E5 41 57 41 56 41 55 41 89 D5 41 54 49 89 F4 53 48 89 FB 48 83 EC 68 \
    F3 0F 10 47 08 F3 0F 10 0D ?? ?? ?? ?? F3 0F 58 C8 E8 ?? ?? ?? ?? F3 0F 10 0D ?? ?? ?? ?? \
    4C 89 E7 0F 28 D0 F3 41 0F 10 44 24 08 F3 0F 5C 53 08",
        " | ",
        "55 48 89 E5 41 57 41 56 41 89 D6 41 55 41 54 49 89 F4 53 48 89 FB 48 83 EC 68 F3 0F 10 47 08 F3 0F 10 0D ?? ?? ?? ?? F3 0F 58 C8 E8 ?? ?? ?? ?? F3 0F 10 0D ?? ?? ?? ?? 4C 89 E7 0F 28 D0 F3 41 0F 10 44 24 08 F3 0F 5C 53 08",
    );
    pub(crate) const DISPATCH_EFFECT_SIG: &str = concat!(
        "55 48 89 E5 41 56 41 55 4C 8D 35 ?? ?? ?? ?? 49 89 FD 41 54 4C 8D 65 A0 53 4C 89 E7 \
    48 89 F3 48 83 EC 40 E8 ?? ?? ?? ?? 48 89 DE 4C 89 E7",
        " | ",
        "55 48 89 E5 41 55 49 89 FD 41 54 49 89 F4 53 48 8D 5D A0 48 89 DF 48 83 EC 48",
    );
    pub(crate) const TE_CREATE_SIG: &str =
        concat!(
        "55 48 89 E5 41 57 41 56 4C 8D 35 ?? ?? ?? ?? 41 55 41 54 4C 8D 25 ?? ?? ?? ?? 53 \
    48 89 FB 48 83 EC 28 48 8D 05 ?? ?? ?? ?? 44 8B 05 ?? ?? ?? ?? 4C 8B 28",
        " | ",
        "55 48 89 E5 41 57 41 56 41 55 49 89 F5 41 54 53 48 89 FB 48 83 EC 18 F3 0F 11 45 CC 4C 8B 25 ?? ?? ?? ?? 49 8B 04 24 48 8B 00 4C 8B B8 E8 01 00 00",
    );
    pub(crate) const PHYSENV_SIG: &str =
        concat!(
        "48 8B 3D ?? ?? ?? ?? 48 8D 95 ?? ?? ?? ?? 4C 89 E6 48 8B 07 FF 50 60",
        " | ",
        "4C 8B 3D ?? ?? ?? ?? 48 8D 85 ?? ?? ?? ?? 45 31 F6 48 89 85 ?? ?? ?? ?? 49 8D 45 0C 4C 8D A5 ?? ?? ?? ?? 48 89 85 ?? ?? ?? ?? 0F 1F 40 00 BA 00 02 00 00 4C 89 E6 48 89 DF E8 ?? ?? ?? ?? 49 8B 3F 4C 89 E6 48 8B 07 FF 50 60",
    );
    pub(crate) const ENTITY_ARRAY_SIG: &str =
        concat!(
        "48 8D 15 ?? ?? ?? ?? 8B 00 48 8B 12 83 F8 FF 74 ?? 89 C1 C1 E8 0E 81 E1 FF 3F 00 00 \
    48 C1 E1 05",
        " | ",
        "48 8B 05 ?? ?? ?? ?? 48 8B 10 8B ?? ?? ?? 00 00 83 F8 FF 74 ?? 89 C1 C1 E8 0E 81 E1 FF 3F 00 00 48 C1 E1 05 48 01 D1",
    );
    /// The server's inline `Ray_t` setup (`UTIL_TraceLine`-style): `m_IsRay`, `m_IsSwept`, and
    /// the `lea rax,[ray]; mov rsi,rax` that passes the ray to `TraceRay`.
    pub(crate) const RAY_INIT_SIG: &str =
        concat!(
        "C6 85 ?? ?? ?? ?? 01 F3 0F 59 D2 F3 0F 11 8D ?? ?? ?? ?? F3 0F 59 C9 F3 0F 58 C2 \
    F3 0F 58 C1 66 0F EF C9 0F 2E C1 0F 9A C0 0F 45 C2 31 C9 31 D2 88 85 ?? ?? ?? ?? E8 \
    ?? ?? ?? ?? 4D 89 E8 48 89 D9 BA 0B 40 01 02 48 8D 05 ?? ?? ?? ?? 48 8B 38 \
    48 8D 85 ?? ?? ?? ?? 48 89 C6",
        " | ",
        "C6 85 ?? ?? ?? ?? 01 F3 0F 11 85 ?? ?? ?? ?? F3 0F 59 C0 F3 0F 58 C2 F3 0F 58 C1 0F 2F C6 0F 95 85 ?? ?? ?? ?? 31 C9 31 D2 E8 ?? ?? ?? ?? 48 8B 05 ?? ?? ?? ?? 4C 89 E1 4C 8B 85 ?? ?? ?? ?? 48 8D B5 ?? ?? ?? ?? BA 0B 40 01 02 48 89 B5 ?? ?? ?? ?? 48 8B 38",
    );
    /// The same in `client.so` (two sites, which must agree).
    pub(crate) const CLIENT_RAY_INIT_SIG: &str =
        concat!(
        "C6 85 ?? ?? ?? ?? 01 F3 0F 58 C4 F3 0F 58 C1 66 0F EF C9 0F 2E C1 0F 9A C0 0F 45 C2 \
    31 C9 31 D2 88 85 ?? ?? ?? ?? E8 ?? ?? ?? ?? 48 8D 05 ?? ?? ?? ?? 48 89 D9 BA 0B 40 ?? ?? \
    4C 8D 85 ?? ?? ?? ?? 4C 89 85 ?? ?? ?? ?? 48 8B 38 48 8D 85 ?? ?? ?? ?? 48 89 C6",
        " | ",
        "F3 0F 11 B5 ?? ?? ?? ?? F3 0F 5C D9 F3 0F 11 AD ?? ?? ?? ?? F3 0F 11 8D ?? ?? ?? ?? F3 0F 58 C4 C7 85 ?? ?? ?? ?? 00 00 00 00 F3 0F 11 9D ?? ?? ?? ?? F3 0F 59 DB C7 85 ?? ?? ?? ?? 00 00 00 00 C7 85 ?? ?? ?? ?? 00 00 00 00 C7 85 ?? ?? ?? ?? 00 00 00 00 F3 0F 58 C3 C7 85 ?? ?? ?? ?? 00 00 00 00 C7 85 ?? ?? ?? ?? 00 00 00 00 48 C7 85 ?? ?? ?? ?? 00 00 00 00 0F 2F C7 C6 85 ?? ?? ?? ?? 01 0F 95 85 ?? ?? ?? ?? 31 C9 31 D2 E8 ?? ?? ?? ??",
    );
    pub(crate) const EXPLOSION_TARGETS: [&[&str]; 3] = [
        &[
            "55 48 89 E5 41 56 41 55 41 54 53 48 89 FB 48 81 EC 90 00 00 00 80 BF ?? 02 00 00 00 \
         0F 84 ?? ?? ?? ?? 48 8B 07 66 0F EF C0 48 C7 45 80 00 00 00 00",
            "55 48 89 E5 41 56 41 55 41 54 53 48 89 FB 48 81 EC B0 00 00 00 80 BF ?? 02 00 00 00 \
         0F 85 ?? ?? ?? ??",
        ],
        &[
            "55 48 89 E5 41 54 53 48 89 FB 48 81 EC A0 00 00 00 80 BF ?? 02 00 00 01 \
         0F 86 ?? ?? ?? ?? 48 8B 07 66 0F EF C0 48 C7 45 90 00 00 00 00",
            "55 48 89 E5 41 54 53 48 89 FB 48 81 EC A0 00 00 00 0F B6 87 ?? 02 00 00 83 F8 01",
        ],
        &[
            "55 48 89 E5 41 57 41 56 41 55 41 54 53 48 89 FB 48 81 EC A8 00 00 00 48 8B 3D \
         ?? ?? ?? ?? 48 8B 47 78 48 85 C0 74 ?? 80 38 00 0F 85 ?? ?? ?? ??",
            "55 48 8D 05 ?? ?? ?? ?? 48 89 E5 41 57 41 56 41 55 41 54 53 48 89 FB 48 81 EC C8 00 \
         00 00 48 8B 3D ?? ?? ?? ?? 48 39 C7 0F 84 ?? ?? ?? ?? 48 8B 07 FF 50 78",
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

    /// The address of a global variable that code reaches with `mov r64,[rip+x]` or
    /// `lea r64,[rip+x]` at `slot`.
    ///
    /// The x86-64 branch's modules are built so that these variables go through the GOT: the
    /// `mov` loads the variable's *address* from a slot the loader filled in (read-only after
    /// loading, so inside `PT_GNU_RELRO`). Other builds address the variable directly.
    pub(crate) unsafe fn resolve_global(slot: usize) -> usize {
        unsafe {
            unsafe extern "C" fn find(
                info: *mut libc::dl_phdr_info,
                _size: usize,
                data: *mut c_void,
            ) -> c_int {
                unsafe {
                    let slot = *data.cast::<usize>();
                    let info = &*info;
                    for i in 0..usize::from(info.dlpi_phnum) {
                        let ph = &*info.dlpi_phdr.add(i);
                        if ph.p_type == libc::PT_GNU_RELRO {
                            let start = info.dlpi_addr as usize + ph.p_vaddr as usize;
                            if (start..start + ph.p_memsz as usize).contains(&slot) {
                                return 1;
                            }
                        }
                    }
                    0
                }
            }
            let mut slot_in = slot;
            if libc::dl_iterate_phdr(Some(find), (&raw mut slot_in).cast::<c_void>()) == 1 {
                *(slot as *const usize)
            } else {
                slot
            }
        }
    }

    /// Load address of the 64-bit Linux server module (`server_srv.so`, `server.so`, or the game
    /// client's `server_client.so`).
    #[cfg(feature = "server")]
    pub(crate) unsafe fn server_base() -> *const u8 {
        unsafe {
            loaded_module(&[b"/server_srv.so", b"/server.so", b"/server_client.so"])
                .map_or(std::ptr::null(), |(base, _)| base as *const u8)
        }
    }

    /// Load address of the 64-bit Linux client module (`client.so`, or `client_client.so` on the x86-64 branch).
    #[cfg(feature = "client")]
    pub(crate) unsafe fn client_base() -> *const u8 {
        unsafe {
            loaded_module(&[b"/client.so", b"/client_client.so"])
                .map_or(std::ptr::null(), |(base, _)| base as *const u8)
        }
    }

    #[cfg(feature = "client")]
    /// An interface of the client module (`CreateInterface("VClientEntityList003")`).
    pub(crate) unsafe fn client_interface(name: &CStr) -> usize {
        unsafe {
            use libloading::os::unix::{Library, RTLD_LAZY};
            use std::ffi::c_char;
            let Some((_, path)) = loaded_module(&[b"/client.so", b"/client_client.so"]) else {
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

    /// The folder that makes `gmod`'s fixed, relative `bin/linux64/lua_shared.so` resolve, from the
    /// full path of the loaded `lua_shared.so`: the game's `garrysmod` folder, in which the
    /// 64-bit client and a listen server keep it (a dedicated server keeps it in its own root).
    pub(crate) fn lua_shared_dir(loaded: &[u8]) -> Option<std::path::PathBuf> {
        use std::os::unix::ffi::OsStrExt;
        const RELATIVE: &[u8] = b"bin/linux64/lua_shared.so";
        let dir = loaded.strip_suffix(RELATIVE)?;
        if !dir.is_empty() && !dir.ends_with(b"/") {
            return None;
        }
        Some(std::path::PathBuf::from(std::ffi::OsStr::from_bytes(dir)))
    }

    /// Runs `load` with `bin/linux64/lua_shared.so` resolving from the working directory, for
    /// the library the game loaded as `loaded` (the full path of its file).
    ///
    /// `gmod` only ever opens that one relative path. The 64-bit client and a listen server
    /// keep the file in `garrysmod/`, so the working directory is switched there. A dedicated
    /// server names its copy `lua_shared_srv.so`, which that path never matches, so a temporary
    /// folder holds a symlink of the expected name to it: the loader recognises a file it has
    /// already loaded by identity, whatever it is opened as, and hands back the loaded library.
    /// The working directory is always put back (the game's other threads use it) and the
    /// temporary folder removed.
    pub(crate) unsafe fn with_lua_shared_visible<T>(
        loaded: &[u8],
        load: impl FnOnce() -> T + std::panic::UnwindSafe,
    ) -> Result<T, String> {
        use std::os::unix::ffi::OsStrExt;
        let target = std::path::Path::new(std::ffi::OsStr::from_bytes(loaded));
        let mut temporary = None;
        let dir = if let Some(dir) = lua_shared_dir(loaded) {
            dir
        } else {
            let dir = std::env::temp_dir().join(format!("waterleveler-{}", std::process::id()));
            let bin = dir.join("bin/linux64");
            std::fs::create_dir_all(&bin)
                .map_err(|e| format!("cannot make {}: {e}", bin.display()))?;
            temporary = Some(dir.clone());
            std::os::unix::fs::symlink(target, bin.join("lua_shared.so"))
                .map_err(|e| format!("cannot link {}: {e}", target.display()))?;
            dir
        };
        let result = (|| {
            let previous =
                std::env::current_dir().map_err(|e| format!("no working directory: {e}"))?;
            std::env::set_current_dir(&dir)
                .map_err(|e| format!("cannot enter {}: {e}", dir.display()))?;
            let result = std::panic::catch_unwind(load);
            let _ = std::env::set_current_dir(previous);
            result.map_err(|_| format!("cannot load {}", target.display()))
        })();
        if let Some(dir) = temporary {
            let _ = std::fs::remove_dir_all(dir);
        }
        result
    }

    /// Loads the Lua API from the Lua library the game already has loaded: `lua_shared.so`,
    /// a dedicated server's `lua_shared_srv.so`, or the game client's `lua_shared_client.so`.
    ///
    /// The loaded one is always preferred, even when `gmod`'s relative
    /// `bin/linux64/lua_shared.so` also exists: that can be another build's copy (the
    /// dedicated server's, next to the client's), and loading it would give a second Lua
    /// library instead of the game's. See [`with_lua_shared_visible`]. Only when none is loaded
    /// does the relative path get its chance. A failed lookup would panic out of
    /// `gmod13_open` and abort the game, so every failure is returned instead.
    pub(crate) unsafe fn load_lua_shared() -> Result<(), String> {
        unsafe {
            if let Some((_, loaded)) = loaded_module(&[
                b"/lua_shared.so",
                b"/lua_shared_srv.so",
                b"/lua_shared_client.so",
            ]) {
                return with_lua_shared_visible(loaded.to_bytes(), || gmod::lua::load());
            }
            if std::path::Path::new("bin/linux64/lua_shared.so").exists() {
                gmod::lua::load();
                return Ok(());
            }
            Err("lua_shared.so is not loaded and not in the working directory".into())
        }
    }

    /// True when this library was loaded as the client module (file name gmcl_*).
    #[cfg(all(feature = "server", feature = "client"))]
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
