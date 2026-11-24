// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67

use crate::*;
use std::ffi::{c_int, c_void, CStr, CString};

pub(crate) const DEFAULT_SAVE_KEY: &str = "m_nWaterLevel";

pub(crate) const HOOK_ID: &CStr = c"gmsv_waterlevel";

pub(crate) const ORIG_KEY: &CStr = c"__waterlevel_orig";

pub(crate) fn hook_hits() -> u64 {
    #[cfg(target_arch = "x86_64")]
    {
        engine_hook::HITS.load(std::sync::atomic::Ordering::Relaxed)
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        0
    }
}

// ---------------------------------------------------------------------------
// Lua helpers
// ---------------------------------------------------------------------------

/// Returns the entity index of the Entity userdata at absolute stack index `idx`.
/// Rejects non-userdata, NULL entities and the world.
pub(crate) unsafe fn ent_index(l: LuaState, idx: c_int) -> Option<i32> {
    unsafe {
        let a = api();
        if (a.type_)(l, idx) != LUA_TUSERDATA {
            return None;
        }
        let top = (a.gettop)(l);
        (a.getfield)(l, idx, c"EntIndex".as_ptr());
        (a.pushvalue)(l, idx);
        let ok = (a.pcall)(l, 1, 1, 0) == 0;
        let n = if ok && (a.type_)(l, -1) == LUA_TNUMBER {
            Some((a.tonumber)(l, -1) as i32)
        } else {
            None
        };
        (a.settop)(l, top);
        n.filter(|&i| i > 0)
    }
}

/// The value of the `developer` convar (0 when it cannot be read).
pub(crate) unsafe fn developer_level(l: LuaState) -> f64 {
    unsafe {
        let a = api();
        let base = (a.gettop)(l);
        let mut level = 0.0;
        (a.getfield)(l, LUA_GLOBALSINDEX, c"GetConVar".as_ptr());
        (a.pushstring)(l, c"developer".as_ptr());
        // The convar object is a userdata; its GetInt is looked up on its metatable.
        if (a.pcall)(l, 1, 1, 0) == 0 && (a.type_)(l, -1) == LUA_TUSERDATA {
            (a.getfield)(l, -1, c"GetInt".as_ptr());
            (a.pushvalue)(l, -2);
            if (a.pcall)(l, 1, 1, 0) == 0 {
                level = (a.tonumber)(l, -1);
            }
        }

        (a.settop)(l, base);
        level
    }
}

pub(crate) unsafe fn lua_print(l: LuaState, msg: &str) {
    unsafe {
        let a = api();
        let Ok(text) = CString::new(msg) else { return };
        let base = (a.gettop)(l);
        (a.getfield)(l, LUA_GLOBALSINDEX, c"print".as_ptr());
        (a.pushstring)(l, text.as_ptr());
        let _ = (a.pcall)(l, 1, 0, 0);
        (a.settop)(l, base);
    }
}

/// Looks the entity up by index and, if still valid, writes the level into its
/// datamap (when `key` is set). Returns false if the entity no longer exists.
pub(crate) unsafe fn apply_level(l: LuaState, idx: i32, level: u8, key: Option<&CStr>) -> bool {
    unsafe {
        let a = api();
        let base = (a.gettop)(l);

        (a.getfield)(l, LUA_GLOBALSINDEX, c"Entity".as_ptr());
        (a.pushnumber)(l, f64::from(idx));
        if (a.pcall)(l, 1, 1, 0) != 0 {
            (a.settop)(l, base);
            return false;
        }
        let ent = base + 1;

        (a.getfield)(l, ent, c"IsValid".as_ptr());
        (a.pushvalue)(l, ent);
        let valid = (a.pcall)(l, 1, 1, 0) == 0 && (a.toboolean)(l, -1) != 0;
        (a.settop)(l, ent);
        if !valid {
            (a.settop)(l, base);
            return false;
        }

        if let Some(key) = key {
            (a.getfield)(l, ent, c"SetSaveValue".as_ptr());
            (a.pushvalue)(l, ent);
            (a.pushstring)(l, key.as_ptr());
            (a.pushnumber)(l, f64::from(level));
            let _ = (a.pcall)(l, 3, 0, 0); // best effort
        }
        (a.settop)(l, base);
        true
    }
}

// ---------------------------------------------------------------------------
// Lua-callable functions
// ---------------------------------------------------------------------------

/// waterleveler.Set(ent, level) / ent:SetWaterLevelOverride(level)
pub(crate) unsafe extern "C-unwind" fn l_set(l: LuaState) -> c_int {
    unsafe {
        let a = api();
        let Some(idx) = ent_index(l, 1) else {
            return fail(l, c"invalid entity");
        };
        if (a.type_)(l, 2) != LUA_TNUMBER {
            return fail(l, c"level must be a number");
        }
        let level = (a.tonumber)(l, 2);
        if !level.is_finite() || level.fract() != 0.0 || !(0.0..=3.0).contains(&level) {
            return fail(l, c"level must be an integer from 0 to 3");
        }
        let level = level as u8;
        overrides().insert(idx, level);
        apply_level(l, idx, level, save_key().as_deref());
        (a.pushboolean)(l, 1);
        1
    }
}

/// waterleveler.Get(ent) / `ent:GetWaterLevelOverride()`
pub(crate) unsafe extern "C-unwind" fn l_get(l: LuaState) -> c_int {
    unsafe {
        let a = api();
        let level = ent_index(l, 1).and_then(|idx| overrides().get(&idx).copied());
        match level {
            Some(v) => (a.pushnumber)(l, f64::from(v)),
            None => (a.pushnil)(l),
        }
        1
    }
}

/// waterleveler.Clear(ent) / `ent:ClearWaterLevelOverride()`
pub(crate) unsafe extern "C-unwind" fn l_clear(l: LuaState) -> c_int {
    unsafe {
        let a = api();
        let removed = ent_index(l, 1).is_some_and(|idx| overrides().remove(&idx).is_some());
        (a.pushboolean)(l, c_int::from(removed));
        1
    }
}

/// `waterleveler.ClearAll()`
pub(crate) unsafe extern "C-unwind" fn l_clear_all(_l: LuaState) -> c_int {
    overrides().clear();
    0
}

/// waterleveler.SetSaveKey(key) - "" disables the per-tick datamap write.
pub(crate) unsafe extern "C-unwind" fn l_set_save_key(l: LuaState) -> c_int {
    unsafe {
        let a = api();
        let p = (a.tolstring)(l, 1, std::ptr::null_mut());
        if p.is_null() {
            return fail(l, c"key must be a string");
        }
        let key = CStr::from_ptr(p).to_string_lossy().into_owned();
        *SAVE_KEY
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(key);
        (a.pushboolean)(l, 1);
        1
    }
}

/// `waterleveler.IsHooked()`
pub(crate) unsafe extern "C-unwind" fn l_is_hooked(l: LuaState) -> c_int {
    unsafe {
        (api().pushboolean)(l, c_int::from(hook_state::active()));
        1
    }
}

/// `waterleveler.HookHits()`
pub(crate) unsafe extern "C-unwind" fn l_hook_hits(l: LuaState) -> c_int {
    unsafe {
        (api().pushnumber)(l, hook_hits() as f64);
        1
    }
}

/// waterleveler.AttachFluid(physobj, nx, ny, nz, dist [, damping] [, options]) -> id | nil, err
/// Makes `physobj` an engine fluid (buoyancy + drag for props inside it).
/// The plane is in world space with the normal pointing out of the fluid;
/// for a water volume that is (0, 0, 1) and the world Z of its top face.
/// `options` is an optional table (it may also be the 6th argument): damping,
/// torque, viscosity, aerodynamics (boolean), contents ("water"/"slime"), currentx, currenty, currentz.
/// Defaults are those of a map's water brush (see `FluidSettings`).
pub(crate) unsafe extern "C-unwind" fn l_attach_fluid(l: LuaState) -> c_int {
    unsafe {
        let a = api();
        if (a.type_)(l, 1) != LUA_TUSERDATA {
            return fail(l, c"AttachFluid needs a PhysObj as its first argument");
        }
        // GMod userdata block: { void *data; u8 type }, PhysObj is type 12.
        let ud = (a.touserdata)(l, 1) as *const *mut c_void;
        if ud.is_null() || *ud.add(1).cast::<u8>() != 12 {
            return fail(l, c"first argument is not a PhysObj");
        }
        let obj = ud.read();
        if obj.is_null() {
            return fail(l, c"PhysObj is invalid");
        }
        let mut v = [0f32; 4];
        for (i, slot) in v.iter_mut().enumerate() {
            let idx = i as c_int + 2;
            if (a.type_)(l, idx) != LUA_TNUMBER {
                return fail(
                    l,
                    c"AttachFluid needs 4 numbers after the PhysObj: nx, ny, nz, dist",
                );
            }
            let n = (a.tonumber)(l, idx);
            if !n.is_finite() {
                return fail(l, c"plane values must be finite");
            }
            *slot = n as f32;
        }
        let mut fluid = FluidSettings::default();
        let mut opts = 0;
        if (a.type_)(l, 6) == LUA_TNUMBER {
            fluid.damping = (a.tonumber)(l, 6) as f32;
        } else if (a.type_)(l, 6) == LUA_TTABLE {
            opts = 6;
        }
        if (a.type_)(l, 7) == LUA_TTABLE {
            opts = 7;
        }
        if opts != 0 {
            let num = |name: &CStr, slot: &mut f32| -> bool {
                (a.getfield)(l, opts, name.as_ptr());
                let present = (a.type_)(l, -1) == LUA_TNUMBER;
                if present {
                    *slot = (a.tonumber)(l, -1) as f32;
                }
                (a.settop)(l, -2);
                present
            };
            num(c"damping", &mut fluid.damping);
            num(c"torque", &mut fluid.torque);
            num(c"viscosity", &mut fluid.viscosity);
            num(c"currentx", &mut fluid.current[0]);
            num(c"currenty", &mut fluid.current[1]);
            num(c"currentz", &mut fluid.current[2]);
            (a.getfield)(l, opts, c"aerodynamics".as_ptr());
            fluid.aerodynamics = (a.toboolean)(l, -1) != 0;
            (a.settop)(l, -2);
            (a.getfield)(l, opts, c"contents".as_ptr());
            let liquid = parse_liquid(l, -1);
            (a.settop)(l, -2);
            match liquid {
                Ok(c) => fluid.contents = c,
                Err(msg) => return fail(l, msg),
            }
        }
        let finite = fluid.damping.is_finite()
            && fluid.torque.is_finite()
            && fluid.viscosity.is_finite()
            && fluid.current.iter().all(|c| c.is_finite());
        if !finite {
            return fail(l, c"fluid settings must be finite numbers");
        }
        match hook_state::fluid_attach(obj as usize, v, &fluid) {
            Ok(id) => {
                (a.pushnumber)(l, f64::from(id));
                1
            }
            Err(why) => {
                let msg = CString::new(why).unwrap_or_default();
                fail(l, &msg)
            }
        }
    }
}

/// waterleveler.DetachFluid(id) -> bool. Call before removing the entity.
pub(crate) unsafe extern "C-unwind" fn l_detach_fluid(l: LuaState) -> c_int {
    unsafe {
        let a = api();
        let ok =
            (a.type_)(l, 1) == LUA_TNUMBER && hook_state::fluid_detach((a.tonumber)(l, 1) as u32);
        (a.pushboolean)(l, c_int::from(ok));
        1
    }
}

/// `waterleveler.FluidCount()`
pub(crate) unsafe extern "C-unwind" fn l_fluid_count(l: LuaState) -> c_int {
    unsafe {
        (api().pushnumber)(l, hook_state::fluid_count() as f64);
        1
    }
}

/// `waterleveler.BubbleClips()` - bullet bubble trails clipped to a volume
pub(crate) unsafe extern "C-unwind" fn l_bubble_clips(l: LuaState) -> c_int {
    unsafe {
        #[cfg(target_arch = "x86_64")]
        let n = engine_hook::BUBBLE_CLIPS.load(std::sync::atomic::Ordering::Relaxed);
        #[cfg(not(target_arch = "x86_64"))]
        let n = 0u64;
        (api().pushnumber)(l, n as f64);
        1
    }
}

/// `waterleveler.ExplosionFixes()` - explosives whose water level was raised
pub(crate) unsafe extern "C-unwind" fn l_explosion_fixes(l: LuaState) -> c_int {
    unsafe {
        #[cfg(target_arch = "x86_64")]
        let n = engine_hook::EXPLOSION_FIXES.load(std::sync::atomic::Ordering::Relaxed);
        #[cfg(not(target_arch = "x86_64"))]
        let n = 0u64;
        (api().pushnumber)(l, n as f64);
        1
    }
}

/// `waterleveler.ExplosionsRedirected()` - explosion events sent as the water explosion
pub(crate) unsafe extern "C-unwind" fn l_explosions_redirected(l: LuaState) -> c_int {
    unsafe {
        #[cfg(target_arch = "x86_64")]
        let n = engine_hook::EXPLOSIONS_REDIRECTED.load(std::sync::atomic::Ordering::Relaxed);
        #[cfg(not(target_arch = "x86_64"))]
        let n = 0u64;
        (api().pushnumber)(l, n as f64);
        1
    }
}

/// Replacement for `Entity:WaterLevel()`. Upvalue 1 is the original function.
pub(crate) unsafe extern "C-unwind" fn l_wrapped_water_level(l: LuaState) -> c_int {
    unsafe {
        let a = api();
        if let Some(idx) = ent_index(l, 1) {
            let level = overrides().get(&idx).copied(); // guard dropped here
            if let Some(level) = level {
                (a.pushnumber)(l, f64::from(level));
                return 1;
            }
        }
        let nargs = (a.gettop)(l);
        (a.pushvalue)(l, LUA_GLOBALSINDEX - 1); // upvalue 1
        for i in 1..=nargs {
            (a.pushvalue)(l, i);
        }
        (a.call)(l, nargs, LUA_MULTRET);
        (a.gettop)(l) - nargs
    }
}

/// Hook "Tick": re-apply every override, drop entities that are gone.
pub(crate) unsafe extern "C-unwind" fn l_tick(l: LuaState) -> c_int {
    unsafe {
        let snapshot: Vec<(i32, u8)> = overrides().iter().map(|(k, v)| (*k, *v)).collect();
        if snapshot.is_empty() {
            return 0;
        }
        let key = save_key();
        for (idx, level) in snapshot {
            if !apply_level(l, idx, level, key.as_deref()) {
                overrides().remove(&idx);
            }
        }
        0
    }
}

/// Hook "EntityRemoved"(ent, fullUpdate): forget the override immediately so a
/// new entity reusing the index never inherits it.
pub(crate) unsafe extern "C-unwind" fn l_entity_removed(l: LuaState) -> c_int {
    unsafe {
        if let Some(idx) = ent_index(l, 1) {
            overrides().remove(&idx);
        }
        0
    }
}

pub(crate) unsafe fn set_nil(l: LuaState, table: c_int, name: &CStr) {
    unsafe {
        let a = api();
        (a.pushnil)(l);
        (a.setfield)(l, table, name.as_ptr());
    }
}

pub(crate) unsafe fn hook_add(l: LuaState, event: &CStr, f: LuaFn) {
    unsafe {
        let a = api();
        let base = (a.gettop)(l);
        (a.getfield)(l, LUA_GLOBALSINDEX, c"hook".as_ptr());
        (a.getfield)(l, base + 1, c"Add".as_ptr());
        (a.pushstring)(l, event.as_ptr());
        (a.pushstring)(l, HOOK_ID.as_ptr());
        (a.pushcclosure)(l, f, 0);
        let _ = (a.pcall)(l, 3, 0, 0);
        (a.settop)(l, base);
    }
}

pub(crate) unsafe fn hook_remove(l: LuaState, event: &CStr) {
    unsafe {
        let a = api();
        let base = (a.gettop)(l);
        (a.getfield)(l, LUA_GLOBALSINDEX, c"hook".as_ptr());
        (a.getfield)(l, base + 1, c"Remove".as_ptr());
        (a.pushstring)(l, event.as_ptr());
        (a.pushstring)(l, HOOK_ID.as_ptr());
        let _ = (a.pcall)(l, 2, 0, 0);
        (a.settop)(l, base);
    }
}

/// Pushes the Entity metatable and returns true, or leaves the stack unchanged.
pub(crate) unsafe fn push_entity_meta(l: LuaState) -> bool {
    unsafe {
        let a = api();
        let base = (a.gettop)(l);
        (a.getfield)(l, LUA_GLOBALSINDEX, c"FindMetaTable".as_ptr());
        (a.pushstring)(l, c"Entity".as_ptr());
        if (a.pcall)(l, 1, 1, 0) == 0 && (a.type_)(l, base + 1) == LUA_TTABLE {
            true
        } else {
            (a.settop)(l, base);
            false
        }
    }
}

/// Loaded as `gmsv_waterlevelercore`: the server's water, the overrides and the fluids.
pub(crate) unsafe fn open_server(l: LuaState) -> c_int {
    unsafe {
        let a = api();
        let base = (a.gettop)(l);

        // Global `waterleveler` table.
        (a.createtable)(l, 0, 16);
        let t = base + 1;
        set_fn(l, t, c"AddVolume", l_add_volume);
        set_fn(l, t, c"RemoveVolume", l_remove_volume);
        set_fn(l, t, c"ClearVolumes", l_clear_volumes);
        set_fn(l, t, c"VolumeCount", l_volume_count);
        set_fn(l, t, c"PointInVolume", l_point_in_volume);
        set_fn(l, t, c"IsContentsHooked", l_is_contents_hooked);
        set_fn(l, t, c"ContentsHits", l_contents_hits);
        set_fn(l, t, c"TraceHits", l_trace_hits);
        set_fn(l, t, c"SetDebug", l_set_debug);
        set_fn(l, t, c"TakeDebugLog", l_take_debug_log);
        set_fn(l, t, c"BubbleClips", l_bubble_clips);
        set_fn(l, t, c"ExplosionFixes", l_explosion_fixes);
        set_fn(l, t, c"ExplosionsRedirected", l_explosions_redirected);
        set_fn(l, t, c"AttachFluid", l_attach_fluid);
        set_fn(l, t, c"DetachFluid", l_detach_fluid);
        set_fn(l, t, c"FluidCount", l_fluid_count);
        set_fn(l, t, c"Set", l_set);
        set_fn(l, t, c"Get", l_get);
        set_fn(l, t, c"Clear", l_clear);
        set_fn(l, t, c"ClearAll", l_clear_all);
        set_fn(l, t, c"SetSaveKey", l_set_save_key);
        set_fn(l, t, c"IsHooked", l_is_hooked);
        set_fn(l, t, c"HookStatus", l_hook_status);
        set_fn(l, t, c"HookHits", l_hook_hits);
        (a.setfield)(l, LUA_GLOBALSINDEX, c"waterlevelercore".as_ptr()); // pops the table

        // Entity metatable: methods + Entity:WaterLevel wrapper.
        if push_entity_meta(l) {
            let mt = base + 1;

            // Keep the original across module reloads.
            (a.getfield)(l, mt, ORIG_KEY.as_ptr());
            let have_orig = (a.type_)(l, -1) != LUA_TNIL;
            (a.settop)(l, mt);
            if !have_orig {
                (a.getfield)(l, mt, c"WaterLevel".as_ptr());
                (a.setfield)(l, mt, ORIG_KEY.as_ptr()); // pops
            }
            (a.getfield)(l, mt, ORIG_KEY.as_ptr());
            (a.pushcclosure)(l, l_wrapped_water_level, 1); // pops the upvalue
            (a.setfield)(l, mt, c"WaterLevel".as_ptr());

            set_fn(l, mt, c"SetWaterLevelOverride", l_set);
            set_fn(l, mt, c"GetWaterLevelOverride", l_get);
            set_fn(l, mt, c"ClearWaterLevelOverride", l_clear);
            (a.settop)(l, base);
        }

        hook_add(l, c"Tick", l_tick);
        hook_add(l, c"EntityRemoved", l_entity_removed);

        // Engine hook (best effort; the Lua layers work without it).
        let water_level = hook_state::install();
        let contents = hook_state::install_contents();
        // A hook or patch that failed to install is always reported (where they exist at all:
        // the Windows and Linux servers). The all-good status is only printed with `developer` above 0; it is
        // always available from `waterleveler_status`.
        // Parts of the volumes that did not install are noted in parentheses.
        let failed = cfg!(any(windows, target_os = "linux"))
            && (water_level.is_err() || contents.as_ref().map_or(true, |what| what.contains(" (")));
        let water_level = match water_level {
            Ok(rva) => format!("hooked CBaseEntity::SetWaterLevel at server module+{rva:#x}"),
            Err(why) => format!("engine hook inactive: {why}"),
        };
        let contents = match contents {
            Ok(what) => format!("water volumes active via {what}"),
            Err(why) => format!("water volumes inactive: {why}"),
        };
        let status = format!("{water_level}; {contents}");
        if failed || developer_level(l) > 0.0 {
            lua_print(l, &format!("[wl] {status}"));
        }
        *HOOK_STATUS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = status;

        (a.settop)(l, base);
        0
    }
}

pub(crate) unsafe fn close_server(l: LuaState) -> c_int {
    unsafe {
        let a = api();
        let base = (a.gettop)(l);

        // Fluid controllers hold engine objects; free them while the module is still loaded.
        hook_state::fluid_detach_all();
        // Remove the detour first so the engine never calls into unloaded code.
        hook_state::uninstall();
        hook_state::uninstall_contents();
        overrides().clear();
        volumes().clear();

        hook_remove(l, c"Tick");
        hook_remove(l, c"EntityRemoved");

        if push_entity_meta(l) {
            let mt = base + 1;
            (a.getfield)(l, mt, ORIG_KEY.as_ptr());
            if (a.type_)(l, -1) == LUA_TNIL {
                (a.settop)(l, mt);
            } else {
                (a.setfield)(l, mt, c"WaterLevel".as_ptr()); // pops
            }
            set_nil(l, mt, ORIG_KEY);
            set_nil(l, mt, c"SetWaterLevelOverride");
            set_nil(l, mt, c"GetWaterLevelOverride");
            set_nil(l, mt, c"ClearWaterLevelOverride");
            (a.settop)(l, base);
        }

        (a.pushnil)(l);
        (a.setfield)(l, LUA_GLOBALSINDEX, c"waterlevelercore".as_ptr());

        (a.settop)(l, base);
        0
    }
}
