// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67

use std::ffi::{c_char, c_void, CStr};

pub(crate) unsafe fn find_symbol(name: &CStr) -> *mut c_void {
    unsafe {
        #[link(name = "kernel32")]
        extern "system" {
            fn GetModuleHandleA(module_name: *const c_char) -> *mut c_void;
            fn GetProcAddress(module: *mut c_void, proc_name: *const c_char) -> *mut c_void;
        }
        let module = GetModuleHandleA(c"lua_shared.dll".as_ptr());
        if module.is_null() {
            return std::ptr::null_mut();
        }
        GetProcAddress(module, name.as_ptr())
    }
}
