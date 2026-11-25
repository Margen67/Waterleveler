// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67

/// True when this DLL was loaded as the client module (file name gmcl_*).
#[cfg(windows)]
pub(crate) unsafe fn is_client_module() -> bool {
    use windows_sys::Win32::System::LibraryLoader::{
        GetModuleFileNameA, GetModuleHandleExA, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
        GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
    };
    unsafe {
        let mut module = std::ptr::null_mut();
        if GetModuleHandleExA(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            is_client_module as *const u8,
            &raw mut module,
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
