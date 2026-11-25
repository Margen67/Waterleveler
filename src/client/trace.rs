// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67

use crate::*;
use std::sync::atomic::AtomicUsize;

/// The `enginetrace` pointer variable loaded by the instruction at `at`.
pub(crate) const unsafe fn trace_global_from_load(at: *const u8) -> *const *mut c_void {
    unsafe {
        let disp = at.add(3).cast::<i32>().read_unaligned() as isize;
        at.offset(7 + disp).cast::<*mut c_void>()
    }
}

/// `IClientEntityList` (client realm). Its vtable slot 3 is GetClientEntity(int).
pub(crate) static CLIENT_ENTITY_LIST: AtomicUsize = AtomicUsize::new(0);

pub(crate) const CLIENT_GET_ENTITY_SLOT: usize = 3;
