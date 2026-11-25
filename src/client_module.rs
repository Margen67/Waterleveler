// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67

/// True when this DLL was loaded as the client module (file name gmcl_*).
#[cfg(windows)]
pub(crate) unsafe fn is_client_module() -> bool {
    unsafe {
        #[link(name = "kernel32")]
        extern "system" {
            fn GetModuleHandleExA(flags: u32, addr: *const c_void, out: *mut *mut c_void) -> i32;
            fn GetModuleFileNameA(module: *mut c_void, buf: *mut u8, size: u32) -> u32;
        }
        const FROM_ADDRESS_UNCHANGED: u32 = 0x4 | 0x2;
        let mut module: *mut c_void = std::ptr::null_mut();
        if GetModuleHandleExA(
            FROM_ADDRESS_UNCHANGED,
            is_client_module as *const c_void,
            &mut module,
        ) == 0
        {
            return false;
        }
        let mut buf = [0u8; 520];
        let n = GetModuleFileNameA(module, buf.as_mut_ptr(), buf.len() as u32) as usize;
        let path = String::from_utf8_lossy(&buf[..n.min(buf.len())]).to_lowercase();
        let name = path.rsplit(['\\', '/']).next().unwrap_or("");
        name.starts_with("gmcl_")
    }
}

#[cfg(not(windows))]
pub(crate) unsafe fn is_client_module() -> bool {
    unsafe { crate::os::is_client_module() }
}
