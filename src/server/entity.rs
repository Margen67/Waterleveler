// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67

use crate::*;
use std::sync::atomic::{AtomicUsize, Ordering};

engine_abi_item! {
/// The replacement for `CBaseEntity::SetWaterLevel`, in the engine's calling
/// convention (see `engine_fn!`).
pub(crate) unsafe fn hook_set_water_level(this: *mut c_void, level: c_int) {
    unsafe {
        let original: engine_fn!((*mut c_void, c_int)) =
            std::mem::transmute(TRAMPOLINE.load(Ordering::Relaxed));
        let mut level = level;
        if !this.is_null() {
            if let Some(idx) = entity_index(this as *const u8) {
                // A lock-free table lookup: never blocks inside an engine callback.
                if let Some(o) = override_get(idx) {
                    level = c_int::from(o);
                    bump(&HITS);
                }
            }
        }
        original(this, level);
    }
}
}

/// Address of the `enginetrace` pointer variable referenced by
/// The `enginetrace` variable that `UpdateWaterState` loads (`func` = start of that function).
pub(crate) unsafe fn trace_global_from(
    func: *const u8,
) -> Result<*const *mut c_void, &'static str> {
    unsafe {
        let code = decode_match(func, UPDATE_WATER_STATE_SIG);
        last_global(&code).map(|global| global as *const *mut c_void)
    }
}

pub(crate) static ENTITY_ARRAY_GLOBAL: AtomicUsize = AtomicUsize::new(0);

/// Address of the global holding the entity array, if every match of the
/// idiom agrees on it.
pub(crate) unsafe fn entity_array_global(base: *const u8) -> Result<usize, &'static str> {
    unsafe {
        let all = scan_sig_all(base, ENTITY_ARRAY_SIG)?;
        let mut global = 0usize;
        for at in all {
            let g = only_global(&decode_match(at, ENTITY_ARRAY_SIG))?;
            if global != 0 && global != g {
                return Err("entity array idiom resolves to different globals");
            }
            global = g;
        }
        if global == 0 {
            return Err("entity array idiom not found");
        }
        Ok(global)
    }
}

// The entity layout of the running game build. It differs between builds (the dev branch's
// server has everything 8 bytes further on than the default branch's), so both platforms
// read it from the signature matches (`learn_*`); until then, the platform's constants
// are used.
pub(crate) static WATER_LEVEL_OFFSET: AtomicUsize = AtomicUsize::new(ENTITY_WATER_LEVEL_OFFSET);

pub(crate) static ORIGIN_OFFSET: AtomicUsize = AtomicUsize::new(ENTITY_ORIGIN_OFFSET);

pub(crate) static EFLAGS_OFFSET: AtomicUsize = AtomicUsize::new(ENTITY_EFLAGS_OFFSET);

pub(crate) static EVENT_SLOT: AtomicUsize = AtomicUsize::new(ENTITY_EVENT_SLOT);

/// `m_vecAbsOrigin` is this far after `m_nWaterLevel` in `CBaseEntity` (0x2ab -> 0x33c in the
/// default Windows build, 0x2d3 -> 0x364 in the dev Linux build).
pub(crate) const ORIGIN_AFTER_WATER_LEVEL: usize = 0x91;

/// Raises `this`'s water level to 2 if its origin is in a volume.
pub(crate) unsafe fn raise_water_level_in_volume(this: *mut u8) {
    unsafe {
        if this.is_null() {
            return;
        }
        let p = read_vec3(
            this.add(ORIGIN_OFFSET.load(Ordering::Relaxed))
                .cast::<f32>(),
        );
        // try_lock: never wait (the original calls back into the contents hook).
        let inside = snapshot_if(|v| v.may_contain(p)).is_some_and(|v| v.contains(p));
        if inside {
            let level = this.add(WATER_LEVEL_OFFSET.load(Ordering::Relaxed));
            if level.read() < 2 {
                level.write(2);
                bump(&EXPLOSION_FIXES);
            }
        }
    }
}
