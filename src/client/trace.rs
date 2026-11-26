// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67

use crate::*;
use std::sync::atomic::AtomicUsize;

/// The `enginetrace` pointer variable loaded by the instruction at `at`.
pub(crate) unsafe fn trace_global_from_load(
    at: *const u8,
) -> Result<*const *mut c_void, &'static str> {
    unsafe {
        let code = decode_match(at, CLIENT_TRACE_SIG);
        last_global(&code).map(|global| global as *const *mut c_void)
    }
}

/// `IClientEntityList` (client realm). Its vtable slot 3 is GetClientEntity(int).
pub(crate) static CLIENT_ENTITY_LIST: AtomicUsize = AtomicUsize::new(0);

pub(crate) const CLIENT_GET_ENTITY_SLOT: usize = 3;
