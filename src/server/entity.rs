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
                // Never block inside an engine callback. No overrides (the usual
                // case): nothing to look up, so no lock either.
                if OVERRIDE_COUNT.load(Ordering::Relaxed) == 0 {
                    original(this, level);
                    return;
                }
                if let Ok(map) = OVERRIDES.try_lock() {
                    if let Some(&o) = map.get(&idx) {
                        level = c_int::from(o);
                        bump(&HITS);
                    }
                }
            }
        }
        original(this, level);
    }
}
}

/// Address of the `enginetrace` pointer variable referenced by
/// `UpdateWaterState` (`func` = start of that function).
pub(crate) const unsafe fn trace_global_from(func: *const u8) -> *const *mut c_void {
    unsafe {
        let disp = func
            .add(TRACE_GLOBAL_DISP_OFFSET)
            .cast::<i32>()
            .read_unaligned() as isize;
        func.offset(TRACE_GLOBAL_DISP_OFFSET as isize + 4 + disp)
            .cast::<*mut c_void>()
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
            let disp = (at.add(ENTITY_ARRAY_DISP_AT) as *const i32).read_unaligned() as isize;
            let g = at.offset((ENTITY_ARRAY_DISP_AT + 4) as isize + disp) as usize;
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
// Windows server has everything 8 bytes further on than the default branch's), so the
// Windows builds read it from the signature matches (`learn_*`); until then, and always on
// Linux, the constants above are used.
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
        let o = this.add(ORIGIN_OFFSET.load(Ordering::Relaxed)) as *const f32;
        let p = [
            o.read_unaligned(),
            o.add(1).read_unaligned(),
            o.add(2).read_unaligned(),
        ];
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
