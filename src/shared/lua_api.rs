// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67

use crate::*;
use gmod::lua::{State, LUA_TNIL, LUA_TNONE, LUA_TNUMBER, LUA_TSTRING};
use std::ffi::CStr;
use std::sync::Mutex;

pub(crate) static HOOK_STATUS: Mutex<String> = Mutex::new(String::new());

/// Pushes `nil, msg` and returns the number of results.
pub(crate) unsafe fn fail(l: State, msg: &str) -> i32 {
    unsafe {
        l.push_nil();
        l.push_string(msg);
        2
    }
}

#[cfg(feature = "server")]
/// Pushes a C string (the Lua stack takes it by length, so a `CStr` needs no conversion).
pub(crate) unsafe fn push_cstr(l: State, s: &CStr) {
    unsafe { l.push_binary_string(s.to_bytes()) }
}

/// `waterleveler.HookStatus()`
#[gmod::lua_function]
pub(crate) unsafe fn l_hook_status(l: State) -> i32 {
    let status = HOOK_STATUS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    // The short commit hash: shows which build is really loaded.
    let status = format!("{status} [build {}]", env!("WL_BUILD"));
    unsafe { l.push_string(&status) };
    1
}

/// The optional liquid argument at stack index `idx`: "water" (default, also nil or
/// none) or "slime".
pub(crate) unsafe fn parse_liquid(l: State, idx: i32) -> Result<Contents, &'static str> {
    const BAD: &str = "liquid must be \"water\" or \"slime\"";
    unsafe {
        match l.lua_type(idx) {
            LUA_TNONE | LUA_TNIL => Ok(Contents::WATER),
            LUA_TSTRING => match l
                .get_binary_string(idx)
                .map(<[u8]>::to_ascii_lowercase)
                .as_deref()
            {
                Some(b"water") => Ok(Contents::WATER),
                Some(b"slime") => Ok(Contents::SLIME),
                _ => Err(BAD),
            },
            _ => Err(BAD),
        }
    }
}

/// waterleveler.AddVolume(x1, y1, z1, x2, y2, z2 [, liquid]) -> id | nil, err
/// Adds a world-space box in which the engine sees water, or slime when `liquid` is
/// "slime" ("water" is the default).
#[gmod::lua_function]
pub(crate) unsafe fn l_add_volume(l: State) -> i32 {
    unsafe {
        let mut v = [0f64; 6];
        for (i, slot) in v.iter_mut().enumerate() {
            let idx = i as i32 + 1;
            if l.lua_type(idx) != LUA_TNUMBER {
                return fail(l, "AddVolume needs 6 numbers: x1, y1, z1, x2, y2, z2");
            }
            *slot = l.to_number(idx);
            if !slot.is_finite() {
                return fail(l, "AddVolume coordinates must be finite");
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
            || fail(l, "too many volumes (limit 256)"),
            |id| {
                l.push_number(f64::from(id));
                1
            },
        )
    }
}

/// waterleveler.RemoveVolume(id) -> bool
#[gmod::lua_function]
pub(crate) unsafe fn l_remove_volume(l: State) -> i32 {
    unsafe {
        let id = l.to_number(1) as u32;
        let removed = {
            let mut vols = volumes();
            let before = vols.len();
            vols.retain(|v| v.id != id);
            vols.len() != before
        };
        l.push_boolean(removed);
        1
    }
}

/// `waterleveler.ClearVolumes()`
#[gmod::lua_function]
pub(crate) unsafe fn l_clear_volumes(_l: State) {
    volumes().clear();
}

/// `waterleveler.VolumeCount()`
#[gmod::lua_function]
pub(crate) unsafe fn l_volume_count(l: State) -> i32 {
    unsafe {
        let n = volumes().len();
        l.push_number(n as f64);
        1
    }
}

/// waterleveler.PointInVolume(x, y, z) -> bool (pure lookup, no engine needed)
#[gmod::lua_function]
pub(crate) unsafe fn l_point_in_volume(l: State) -> i32 {
    unsafe {
        let p = [
            l.to_number(1) as f32,
            l.to_number(2) as f32,
            l.to_number(3) as f32,
        ];
        let inside = volumes().contains(p);
        l.push_boolean(inside);
        1
    }
}

/// `waterleveler.IsContentsHooked()`
#[gmod::lua_function]
pub(crate) unsafe fn l_is_contents_hooked(l: State) -> i32 {
    unsafe {
        l.push_boolean(hook_state::contents_active());
        1
    }
}

/// `waterleveler.ContentsHits()` - engine point queries answered with water
#[gmod::lua_function]
pub(crate) unsafe fn l_contents_hits(l: State) -> i32 {
    unsafe {
        let n = VOLUME_HITS.load(std::sync::atomic::Ordering::Relaxed);
        l.push_number(n as f64);
        1
    }
}

/// `waterleveler.SetDebug(1|0)` - log what the hooks answer near a volume (read it with `TakeDebugLog`)
#[gmod::lua_function]
pub(crate) unsafe fn l_set_debug(l: State) {
    unsafe {
        #[cfg(target_arch = "x86_64")]
        DEBUG.store(l.to_number(1) != 0.0, std::sync::atomic::Ordering::Relaxed);
        #[cfg(not(target_arch = "x86_64"))]
        let _ = l;
    }
}

/// `waterleveler.TakeDebugLog()` -> string: the log lines since the last call, empties the log
#[gmod::lua_function]
pub(crate) unsafe fn l_take_debug_log(l: State) -> i32 {
    unsafe {
        #[cfg(target_arch = "x86_64")]
        let text = take_debug_log();
        #[cfg(not(target_arch = "x86_64"))]
        let text = String::new();
        l.push_string(&text);
        1
    }
}

/// `waterleveler.TraceHits()` - engine traces rewritten as water-surface hits
#[gmod::lua_function]
pub(crate) unsafe fn l_trace_hits(l: State) -> i32 {
    unsafe {
        #[cfg(target_arch = "x86_64")]
        let n = TRACE_HITS.load(std::sync::atomic::Ordering::Relaxed);
        #[cfg(not(target_arch = "x86_64"))]
        let n = 0u64;
        l.push_number(n as f64);
        1
    }
}

/// Sets `table[name] = f` for the table at stack index `table`.
pub(crate) unsafe fn set_fn(l: State, table: i32, name: &CStr, f: gmod::lua::LuaFunction) {
    unsafe {
        l.push_function(f);
        l.set_field(table, name.as_ptr());
    }
}
