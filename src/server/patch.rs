// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67

use crate::*;
use detour::RawDetour;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

pub(crate) static TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);

pub(crate) static HITS: AtomicU64 = AtomicU64::new(0);

/// Walks the executable sections of a loaded PE image (`base` = module
/// base) and returns the single match of [`SIG`].
pub(crate) unsafe fn scan_image(base: *const u8) -> Result<*mut u8, &'static str> {
    unsafe { scan_sig(base, SIG) }
}

/// Detours `target` to `hook` with the `detour` crate, which relocates the stolen
/// instructions itself and pauses the other threads while it patches. The address of the
/// original function (the trampoline) is stored in `trampoline` *before* the detour is
/// enabled, so `hook` can call it from its first run. Dropping the detour restores the
/// original bytes.
pub(crate) unsafe fn install_detour(
    target: *mut u8,
    hook: *const (),
    trampoline: &AtomicUsize,
) -> Result<RawDetour, &'static str> {
    unsafe {
        // The error text is leaked: it is read once, to report why a hook is inactive.
        let leak = |e: detour::Error| -> &'static str { Box::leak(e.to_string().into_boxed_str()) };
        let detour = RawDetour::new(target.cast_const().cast(), hook).map_err(leak)?;
        trampoline.store(detour.trampoline() as usize, Ordering::SeqCst);
        detour.enable().map_err(leak)?;
        Ok(detour)
    }
}

#[allow(clippy::missing_const_for_fn)]
pub(crate) unsafe fn read_disp(at: *const u8, i: usize) -> usize {
    unsafe { at.add(i).cast::<u32>().read_unaligned() as usize }
}
