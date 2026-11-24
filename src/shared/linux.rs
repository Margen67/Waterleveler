// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67

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
