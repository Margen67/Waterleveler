// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67

use crate::*;
use std::ffi::{c_int, CStr, CString};
use std::sync::Mutex;

pub(crate) static HOOK_STATUS: Mutex<String> = Mutex::new(String::new());

pub(crate) unsafe fn fail(l: LuaState, msg: &CStr) -> c_int {
    unsafe {
        let a = api();
        (a.pushnil)(l);
        (a.pushstring)(l, msg.as_ptr());
        2
    }
}

/// `waterleveler.HookStatus()`
pub(crate) unsafe extern "C-unwind" fn l_hook_status(l: LuaState) -> c_int {
    unsafe {
        let status = HOOK_STATUS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        // The short commit hash: shows which build is really loaded.
        let status = format!("{status} [build {}]", env!("WL_BUILD"));
        let text = CString::new(status).unwrap_or_default();
        (api().pushstring)(l, text.as_ptr());
        1
    }
}

/// The optional liquid argument at stack index `idx`: "water" (default, also nil or
/// none) or "slime".
pub(crate) unsafe fn parse_liquid(l: LuaState, idx: c_int) -> Result<c_int, &'static CStr> {
    unsafe {
        let a = api();
        match (a.type_)(l, idx) {
            LUA_TNONE | LUA_TNIL => Ok(CONTENTS_WATER),
            LUA_TSTRING => {
                let p = (a.tolstring)(l, idx, std::ptr::null_mut());
                match CStr::from_ptr(p).to_bytes().to_ascii_lowercase().as_slice() {
                    b"water" => Ok(CONTENTS_WATER),
                    b"slime" => Ok(CONTENTS_SLIME),
                    _ => Err(c"liquid must be \"water\" or \"slime\""),
                }
            }
            _ => Err(c"liquid must be \"water\" or \"slime\""),
        }
    }
}

/// waterleveler.AddVolume(x1, y1, z1, x2, y2, z2 [, liquid]) -> id | nil, err
/// Adds a world-space box in which the engine sees water, or slime when `liquid` is
/// "slime" ("water" is the default).
pub(crate) unsafe extern "C-unwind" fn l_add_volume(l: LuaState) -> c_int {
    unsafe {
        let a = api();
        let mut v = [0f64; 6];
        for (i, slot) in v.iter_mut().enumerate() {
            let idx = i as c_int + 1;
            if (a.type_)(l, idx) != LUA_TNUMBER {
                return fail(l, c"AddVolume needs 6 numbers: x1, y1, z1, x2, y2, z2");
            }
            *slot = (a.tonumber)(l, idx);
            if !slot.is_finite() {
                return fail(l, c"AddVolume coordinates must be finite");
            }
        }
        let contents = match parse_liquid(l, 7) {
            Ok(c) => c,
            Err(msg) => return fail(l, msg),
        };
        let min = [
            v[0].min(v[3]) as f32,
            v[1].min(v[4]) as f32,
            v[2].min(v[5]) as f32,
        ];
        let max = [
            v[0].max(v[3]) as f32,
            v[1].max(v[4]) as f32,
            v[2].max(v[5]) as f32,
        ];
        let id = {
            let mut vols = volumes();
            let id = if vols.len() >= MAX_VOLUMES {
                None
            } else {
                let id = NEXT_VOLUME_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                vols.push(Volume {
                    id,
                    min,
                    max,
                    contents,
                });
                Some(id)
            };
            drop(vols);
            id
        };
        id.map_or_else(
            || fail(l, c"too many volumes (limit 256)"),
            |id| {
                (a.pushnumber)(l, f64::from(id));
                1
            },
        )
    }
}

/// waterleveler.RemoveVolume(id) -> bool
pub(crate) unsafe extern "C-unwind" fn l_remove_volume(l: LuaState) -> c_int {
    unsafe {
        let a = api();
        let id = (a.tonumber)(l, 1) as u32;
        let removed = {
            let mut vols = volumes();
            let before = vols.len();
            vols.retain(|v| v.id != id);
            vols.len() != before
        };
        (a.pushboolean)(l, c_int::from(removed));
        1
    }
}

/// `waterleveler.ClearVolumes()`
pub(crate) unsafe extern "C-unwind" fn l_clear_volumes(_l: LuaState) -> c_int {
    volumes().clear();
    0
}

/// `waterleveler.VolumeCount()`
pub(crate) unsafe extern "C-unwind" fn l_volume_count(l: LuaState) -> c_int {
    unsafe {
        let n = volumes().len();
        (api().pushnumber)(l, n as f64);
        1
    }
}

/// waterleveler.PointInVolume(x, y, z) -> bool (pure lookup, no engine needed)
pub(crate) unsafe extern "C-unwind" fn l_point_in_volume(l: LuaState) -> c_int {
    unsafe {
        let a = api();
        let p = [
            (a.tonumber)(l, 1) as f32,
            (a.tonumber)(l, 2) as f32,
            (a.tonumber)(l, 3) as f32,
        ];
        let inside = volumes().contains(p);
        (a.pushboolean)(l, c_int::from(inside));
        1
    }
}

/// `waterleveler.IsContentsHooked()`
pub(crate) unsafe extern "C-unwind" fn l_is_contents_hooked(l: LuaState) -> c_int {
    unsafe {
        (api().pushboolean)(l, c_int::from(hook_state::contents_active()));
        1
    }
}

/// `waterleveler.ContentsHits()` - engine point queries answered with water
pub(crate) unsafe extern "C-unwind" fn l_contents_hits(l: LuaState) -> c_int {
    unsafe {
        let n = VOLUME_HITS.load(std::sync::atomic::Ordering::Relaxed);
        (api().pushnumber)(l, n as f64);
        1
    }
}

/// `waterleveler.SetDebug(1|0)` - log what the hooks answer near a volume (read it with `TakeDebugLog`)
pub(crate) unsafe extern "C-unwind" fn l_set_debug(l: LuaState) -> c_int {
    unsafe {
        #[cfg(target_arch = "x86_64")]
        engine_hook::DEBUG.store(
            (api().tonumber)(l, 1) != 0.0,
            std::sync::atomic::Ordering::Relaxed,
        );
        #[cfg(not(target_arch = "x86_64"))]
        let _ = l;
        0
    }
}

/// `waterleveler.TakeDebugLog()` -> string: the log lines since the last call, empties the log
pub(crate) unsafe extern "C-unwind" fn l_take_debug_log(l: LuaState) -> c_int {
    unsafe {
        #[cfg(target_arch = "x86_64")]
        let text = engine_hook::take_debug_log();
        #[cfg(not(target_arch = "x86_64"))]
        let text = String::new();
        let text = CString::new(text.replace('\0', "")).unwrap_or_default();
        (api().pushstring)(l, text.as_ptr());
        1
    }
}

/// `waterleveler.TraceHits()` - engine traces rewritten as water-surface hits
pub(crate) unsafe extern "C-unwind" fn l_trace_hits(l: LuaState) -> c_int {
    unsafe {
        #[cfg(target_arch = "x86_64")]
        let n = engine_hook::TRACE_HITS.load(std::sync::atomic::Ordering::Relaxed);
        #[cfg(not(target_arch = "x86_64"))]
        let n = 0u64;
        (api().pushnumber)(l, n as f64);
        1
    }
}

// ---------------------------------------------------------------------------
// Module entry points
// ---------------------------------------------------------------------------

pub(crate) unsafe fn set_fn(l: LuaState, table: c_int, name: &CStr, f: LuaFn) {
    unsafe {
        let a = api();
        (a.pushcclosure)(l, f, 0);
        (a.setfield)(l, table, name.as_ptr());
    }
}
