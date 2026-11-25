// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67

use crate::*;
use region::Protection;

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

pub(crate) const PLATFORM: Platform = Platform {
    make_writable,
    restore_protect,
};
