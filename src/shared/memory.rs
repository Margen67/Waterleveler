// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67

use crate::*;
use region::Protection;

/// Executable memory for a trampoline. It is never freed: the patched code can run until
/// the process ends.
#[cfg(feature = "server")]
unsafe fn alloc_exec(size: usize) -> *mut u8 {
    region::alloc(size, Protection::READ_WRITE_EXECUTE).map_or(std::ptr::null_mut(), |mut pages| {
        let p = pages.as_mut_ptr::<u8>();
        std::mem::forget(pages);
        p
    })
}

/// Makes the range writable and returns the protection it had before (as `Protection` bits).
unsafe fn make_writable(p: *mut u8, n: usize) -> Option<u32> {
    unsafe {
        let old = region::query(p).ok()?.protection();
        region::protect(p, n, old | Protection::WRITE).ok()?;
        Some(old.bits() as u32)
    }
}

unsafe fn restore_protect(p: *mut u8, n: usize, old: u32) {
    unsafe {
        let _ = region::protect(p, n, Protection::from_bits_retain(old as usize));
    }
}

#[cfg(feature = "server")]
#[allow(clippy::missing_const_for_fn)] // only const off Windows
unsafe fn flush(p: *mut u8, n: usize) {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::{
            Diagnostics::Debug::FlushInstructionCache, Threading::GetCurrentProcess,
        };
        FlushInstructionCache(GetCurrentProcess(), p.cast(), n);
    }
    // x86 keeps instruction fetches coherent with stores
    #[cfg(not(windows))]
    let _ = (p, n);
}

pub(crate) const PLATFORM: Platform = Platform {
    #[cfg(feature = "server")]
    alloc_exec,
    make_writable,
    restore_protect,
    #[cfg(feature = "server")]
    flush,
};
