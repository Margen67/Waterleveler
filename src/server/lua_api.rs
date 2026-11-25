// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67

use crate::*;
use gmod::lua::{State, LUA_MULTRET, LUA_TNIL, LUA_TNUMBER, LUA_TTABLE, LUA_TUSERDATA};
use std::ffi::{c_void, CStr};

pub(crate) const DEFAULT_SAVE_KEY: &str = "m_nWaterLevel";

pub(crate) const HOOK_ID: &CStr = c"gmsv_waterlevel";

pub(crate) const ORIG_KEY: &CStr = c"__waterlevel_orig";

pub(crate) fn hook_hits() -> u64 {
    #[cfg(target_arch = "x86_64")]
    {
        HITS.load(std::sync::atomic::Ordering::Relaxed)
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
pub(crate) unsafe fn ent_index(l: State, idx: i32) -> Option<i32> {
    unsafe {
        if l.lua_type(idx) != LUA_TUSERDATA {
            return None;
        }
        let top = l.get_top();
        l.get_field(idx, c"EntIndex".as_ptr());
        l.push_value(idx);
        let ok = l.pcall(1, 1, 0) == 0;
        let n = if ok && l.lua_type(-1) == LUA_TNUMBER {
            Some(l.to_number(-1) as i32)
        } else {
            None
        };
        l.set_top(top);
        n.filter(|&i| i > 0)
    }
}

/// The value of the `developer` convar (0 when it cannot be read).
pub(crate) unsafe fn developer_level(l: State) -> f64 {
    unsafe {
        let base = l.get_top();
        let mut level = 0.0;
        l.get_global(c"GetConVar".as_ptr());
        l.push_string("developer");
        // The convar object is a userdata; its GetInt is looked up on its metatable.
        if l.pcall(1, 1, 0) == 0 && l.lua_type(-1) == LUA_TUSERDATA {
            l.get_field(-1, c"GetInt".as_ptr());
            l.push_value(-2);
            if l.pcall(1, 1, 0) == 0 {
                level = l.to_number(-1);
            }
        }

        l.set_top(base);
        level
    }
}

pub(crate) unsafe fn lua_print(l: State, msg: &str) {
    unsafe {
        let base = l.get_top();
        l.get_global(c"print".as_ptr());
        l.push_string(msg);
        let _ = l.pcall(1, 0, 0);
        l.set_top(base);
    }
}

/// Looks the entity up by index and, if still valid, writes the level into its
/// datamap (when `key` is set). Returns false if the entity no longer exists.
pub(crate) unsafe fn apply_level(l: State, idx: i32, level: u8, key: Option<&CStr>) -> bool {
    unsafe {
        let base = l.get_top();

        l.get_global(c"Entity".as_ptr());
        l.push_number(f64::from(idx));
        if l.pcall(1, 1, 0) != 0 {
            l.set_top(base);
            return false;
        }
        let ent = base + 1;

        l.get_field(ent, c"IsValid".as_ptr());
        l.push_value(ent);
        let valid = l.pcall(1, 1, 0) == 0 && l.get_boolean(-1);
        l.set_top(ent);
        if !valid {
            l.set_top(base);
            return false;
        }

        if let Some(key) = key {
            l.get_field(ent, c"SetSaveValue".as_ptr());
            l.push_value(ent);
            push_cstr(l, key);
            l.push_number(f64::from(level));
            let _ = l.pcall(3, 0, 0); // best effort
        }
        l.set_top(base);
        true
    }
}

// ---------------------------------------------------------------------------
// Lua-callable functions
// ---------------------------------------------------------------------------

/// waterleveler.Set(ent, level) / ent:SetWaterLevelOverride(level)
#[gmod::lua_function]
pub(crate) unsafe fn l_set(l: State) -> i32 {
    unsafe {
        let Some(idx) = ent_index(l, 1) else {
            return fail(l, "invalid entity");
        };
        if l.lua_type(2) != LUA_TNUMBER {
            return fail(l, "level must be a number");
        }
        let level = l.to_number(2);
        if !level.is_finite() || level.fract() != 0.0 || !(0.0..=3.0).contains(&level) {
            return fail(l, "level must be an integer from 0 to 3");
        }
        let level = level as u8;
        overrides().insert(idx, level);
        apply_level(l, idx, level, save_key().as_deref());
        l.push_boolean(true);
        1
    }
}

/// waterleveler.Get(ent) / `ent:GetWaterLevelOverride()`
#[gmod::lua_function]
pub(crate) unsafe fn l_get(l: State) -> i32 {
    unsafe {
        let level = ent_index(l, 1).and_then(|idx| overrides().get(&idx).copied());
        match level {
            Some(v) => l.push_number(f64::from(v)),
            None => l.push_nil(),
        }
        1
    }
}

/// waterleveler.Clear(ent) / `ent:ClearWaterLevelOverride()`
#[gmod::lua_function]
pub(crate) unsafe fn l_clear(l: State) -> i32 {
    unsafe {
        let removed = ent_index(l, 1).is_some_and(|idx| overrides().remove(&idx).is_some());
        l.push_boolean(removed);
        1
    }
}

/// `waterleveler.ClearAll()`
#[gmod::lua_function]
pub(crate) unsafe fn l_clear_all(_l: State) {
    overrides().clear();
}

/// waterleveler.SetSaveKey(key) - "" disables the per-tick datamap write.
#[gmod::lua_function]
pub(crate) unsafe fn l_set_save_key(l: State) -> i32 {
    unsafe {
        let Some(key) = l.get_string(1).map(std::borrow::Cow::into_owned) else {
            return fail(l, "key must be a string");
        };
        *SAVE_KEY
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(key);
        l.push_boolean(true);
        1
    }
}

/// `waterleveler.IsHooked()`
#[gmod::lua_function]
pub(crate) unsafe fn l_is_hooked(l: State) -> i32 {
    unsafe {
        l.push_boolean(hook_state::active());
        1
    }
}

/// `waterleveler.HookHits()`
#[gmod::lua_function]
pub(crate) unsafe fn l_hook_hits(l: State) -> i32 {
    unsafe {
        l.push_number(hook_hits() as f64);
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
#[gmod::lua_function]
pub(crate) unsafe fn l_attach_fluid(l: State) -> i32 {
    unsafe { attach_fluid(l) }
}

// The body is a separate function: the `lua_function` macro cannot parse the `c"..."` literals.
unsafe fn attach_fluid(l: State) -> i32 {
    unsafe {
        if l.lua_type(1) != LUA_TUSERDATA {
            return fail(l, "AttachFluid needs a PhysObj as its first argument");
        }
        // GMod userdata block: { void *data; u8 type }, PhysObj is type 12.
        let ud = l.to_userdata(1) as *const *mut c_void;
        if ud.is_null() || *ud.add(1).cast::<u8>() != 12 {
            return fail(l, "first argument is not a PhysObj");
        }
        let obj = ud.read();
        if obj.is_null() {
            return fail(l, "PhysObj is invalid");
        }
        let mut v = [0f32; 4];
        for (i, slot) in v.iter_mut().enumerate() {
            let idx = i as i32 + 2;
            if l.lua_type(idx) != LUA_TNUMBER {
                return fail(
                    l,
                    "AttachFluid needs 4 numbers after the PhysObj: nx, ny, nz, dist",
                );
            }
            let n = l.to_number(idx);
            if !n.is_finite() {
                return fail(l, "plane values must be finite");
            }
            *slot = n as f32;
        }
        let mut fluid = FluidSettings::default();
        let mut opts = 0;
        if l.lua_type(6) == LUA_TNUMBER {
            fluid.damping = l.to_number(6) as f32;
        } else if l.lua_type(6) == LUA_TTABLE {
            opts = 6;
        }
        if l.lua_type(7) == LUA_TTABLE {
            opts = 7;
        }
        if opts != 0 {
            let num = |name: &CStr, slot: &mut f32| -> bool {
                l.get_field(opts, name.as_ptr());
                let present = l.lua_type(-1) == LUA_TNUMBER;
                if present {
                    *slot = l.to_number(-1) as f32;
                }
                l.set_top(-2);
                present
            };
            num(c"damping", &mut fluid.damping);
            num(c"torque", &mut fluid.torque);
            num(c"viscosity", &mut fluid.viscosity);
            num(c"currentx", &mut fluid.current[0]);
            num(c"currenty", &mut fluid.current[1]);
            num(c"currentz", &mut fluid.current[2]);
            l.get_field(opts, c"aerodynamics".as_ptr());
            fluid.aerodynamics = l.get_boolean(-1);
            l.set_top(-2);
            l.get_field(opts, c"contents".as_ptr());
            let liquid = parse_liquid(l, -1);
            l.set_top(-2);
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
            return fail(l, "fluid settings must be finite numbers");
        }
        match hook_state::fluid_attach(obj as usize, v, &fluid) {
            Ok(id) => {
                l.push_number(f64::from(id));
                1
            }
            Err(why) => fail(l, why),
        }
    }
}

/// waterleveler.DetachFluid(id) -> bool. Call before removing the entity.
#[gmod::lua_function]
pub(crate) unsafe fn l_detach_fluid(l: State) -> i32 {
    unsafe {
        let ok = l.lua_type(1) == LUA_TNUMBER && hook_state::fluid_detach(l.to_number(1) as u32);
        l.push_boolean(ok);
        1
    }
}

/// `waterleveler.FluidCount()`
#[gmod::lua_function]
pub(crate) unsafe fn l_fluid_count(l: State) -> i32 {
    unsafe {
        l.push_number(hook_state::fluid_count() as f64);
        1
    }
}

/// `waterleveler.BubbleClips()` - bullet bubble trails clipped to a volume
#[gmod::lua_function]
pub(crate) unsafe fn l_bubble_clips(l: State) -> i32 {
    unsafe {
        #[cfg(target_arch = "x86_64")]
        let n = BUBBLE_CLIPS.load(std::sync::atomic::Ordering::Relaxed);
        #[cfg(not(target_arch = "x86_64"))]
        let n = 0u64;
        l.push_number(n as f64);
        1
    }
}

/// `waterleveler.ExplosionFixes()` - explosives whose water level was raised
#[gmod::lua_function]
pub(crate) unsafe fn l_explosion_fixes(l: State) -> i32 {
    unsafe {
        #[cfg(target_arch = "x86_64")]
        let n = EXPLOSION_FIXES.load(std::sync::atomic::Ordering::Relaxed);
        #[cfg(not(target_arch = "x86_64"))]
        let n = 0u64;
        l.push_number(n as f64);
        1
    }
}

/// `waterleveler.ExplosionsRedirected()` - explosion events sent as the water explosion
#[gmod::lua_function]
pub(crate) unsafe fn l_explosions_redirected(l: State) -> i32 {
    unsafe {
        #[cfg(target_arch = "x86_64")]
        let n = EXPLOSIONS_REDIRECTED.load(std::sync::atomic::Ordering::Relaxed);
        #[cfg(not(target_arch = "x86_64"))]
        let n = 0u64;
        l.push_number(n as f64);
        1
    }
}

/// Replacement for `Entity:WaterLevel()`. Upvalue 1 is the original function.
#[gmod::lua_function]
pub(crate) unsafe fn l_wrapped_water_level(l: State) -> i32 {
    unsafe {
        if let Some(idx) = ent_index(l, 1) {
            let level = overrides().get(&idx).copied(); // guard dropped here
            if let Some(level) = level {
                l.push_number(f64::from(level));
                return 1;
            }
        }
        let nargs = l.get_top();
        l.push_closure_arg(1);
        for i in 1..=nargs {
            l.push_value(i);
        }
        l.call(nargs, LUA_MULTRET);
        l.get_top() - nargs
    }
}

/// Hook "Tick": re-apply every override, drop entities that are gone.
#[gmod::lua_function]
pub(crate) unsafe fn l_tick(l: State) {
    unsafe {
        let snapshot: Vec<(i32, u8)> = overrides().iter().map(|(k, v)| (*k, *v)).collect();
        if snapshot.is_empty() {
            return;
        }
        let key = save_key();
        for (idx, level) in snapshot {
            if !apply_level(l, idx, level, key.as_deref()) {
                overrides().remove(&idx);
            }
        }
    }
}

/// Hook "EntityRemoved"(ent, fullUpdate): forget the override immediately so a
/// new entity reusing the index never inherits it.
#[gmod::lua_function]
pub(crate) unsafe fn l_entity_removed(l: State) {
    unsafe {
        if let Some(idx) = ent_index(l, 1) {
            overrides().remove(&idx);
        }
    }
}

pub(crate) unsafe fn set_nil(l: State, table: i32, name: &CStr) {
    unsafe {
        l.push_nil();
        l.set_field(table, name.as_ptr());
    }
}

pub(crate) unsafe fn hook_add(l: State, event: &CStr, f: gmod::lua::LuaFunction) {
    unsafe {
        let base = l.get_top();
        l.get_global(c"hook".as_ptr());
        l.get_field(base + 1, c"Add".as_ptr());
        push_cstr(l, event);
        push_cstr(l, HOOK_ID);
        l.push_function(f);
        let _ = l.pcall(3, 0, 0);
        l.set_top(base);
    }
}

pub(crate) unsafe fn hook_remove(l: State, event: &CStr) {
    unsafe {
        let base = l.get_top();
        l.get_global(c"hook".as_ptr());
        l.get_field(base + 1, c"Remove".as_ptr());
        push_cstr(l, event);
        push_cstr(l, HOOK_ID);
        let _ = l.pcall(2, 0, 0);
        l.set_top(base);
    }
}

/// Pushes the Entity metatable and returns true, or leaves the stack unchanged.
pub(crate) unsafe fn push_entity_meta(l: State) -> bool {
    unsafe {
        let base = l.get_top();
        l.get_global(c"FindMetaTable".as_ptr());
        l.push_string("Entity");
        if l.pcall(1, 1, 0) == 0 && l.lua_type(base + 1) == LUA_TTABLE {
            true
        } else {
            l.set_top(base);
            false
        }
    }
}

/// Loaded as `gmsv_waterlevelercore`: the server's water, the overrides and the fluids.
pub(crate) unsafe fn open_server(l: State) -> i32 {
    unsafe {
        let base = l.get_top();

        // Global `waterleveler` table.
        l.create_table(0, 16);
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
        l.set_global(c"waterlevelercore".as_ptr()); // pops the table

        // Entity metatable: methods + Entity:WaterLevel wrapper.
        if push_entity_meta(l) {
            let mt = base + 1;

            // Keep the original across module reloads.
            l.get_field(mt, ORIG_KEY.as_ptr());
            let have_orig = l.lua_type(-1) != LUA_TNIL;
            l.set_top(mt);
            if !have_orig {
                l.get_field(mt, c"WaterLevel".as_ptr());
                l.set_field(mt, ORIG_KEY.as_ptr()); // pops
            }
            l.get_field(mt, ORIG_KEY.as_ptr());
            l.push_closure(l_wrapped_water_level, 1); // pops the upvalue
            l.set_field(mt, c"WaterLevel".as_ptr());

            set_fn(l, mt, c"SetWaterLevelOverride", l_set);
            set_fn(l, mt, c"GetWaterLevelOverride", l_get);
            set_fn(l, mt, c"ClearWaterLevelOverride", l_clear);
            l.set_top(base);
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

        l.set_top(base);
        0
    }
}

pub(crate) unsafe fn close_server(l: State) -> i32 {
    unsafe {
        let base = l.get_top();

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
            l.get_field(mt, ORIG_KEY.as_ptr());
            if l.lua_type(-1) == LUA_TNIL {
                l.set_top(mt);
            } else {
                l.set_field(mt, c"WaterLevel".as_ptr()); // pops
            }
            set_nil(l, mt, ORIG_KEY);
            set_nil(l, mt, c"SetWaterLevelOverride");
            set_nil(l, mt, c"GetWaterLevelOverride");
            set_nil(l, mt, c"ClearWaterLevelOverride");
            l.set_top(base);
        }

        l.push_nil();
        l.set_global(c"waterlevelercore".as_ptr());

        l.set_top(base);
        0
    }
}
