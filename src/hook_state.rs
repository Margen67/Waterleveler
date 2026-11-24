// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67
use super::engine_hook;
#[cfg(feature = "server")]
use super::engine_hook::STOLEN;
#[allow(clippy::wildcard_imports)]
// this code needs more names than the other builds use
use super::*;

#[cfg(feature = "server")]
pub(crate) static TARGET: Mutex<Option<(usize, [u8; STOLEN])>> = Mutex::new(None);

#[cfg(feature = "server")]
pub(crate) unsafe fn install() -> Result<usize, &'static str> {
    unsafe {
        let base = engine_hook::os::server_base();
        if base.is_null() {
            return Err("the server module is not loaded");
        }
        let target = engine_hook::scan_image(base)?;
        engine_hook::learn_water_level_layout(target)?;
        let original = engine_hook::install(target, &engine_hook::os::PLATFORM)?;
        *TARGET
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some((target as usize, original));
        Ok(target as usize - base as usize)
    }
}

#[cfg(feature = "server")]
pub(crate) unsafe fn uninstall() {
    unsafe {
        let taken = TARGET
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some((target, original)) = taken {
            engine_hook::uninstall(target as *mut u8, &original, &engine_hook::os::PLATFORM);
        }
    }
}

#[cfg(feature = "server")]
pub(crate) fn active() -> bool {
    TARGET
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .is_some()
}

/// (vtable slot address, original function pointer)
pub(crate) static CONTENTS: Mutex<Vec<(usize, usize, usize)>> = Mutex::new(Vec::new());

#[cfg(feature = "server")]
/// Hooks `IEngineTrace::GetPointContents` via the engine object that
/// `server.dll` itself uses (found through `UpdateWaterState`).
#[allow(clippy::too_many_lines)]
pub(crate) unsafe fn install_contents() -> Result<String, &'static str> {
    unsafe {
        let base = engine_hook::os::server_base();
        if base.is_null() {
            return Err("the server module is not loaded");
        }
        let func = engine_hook::scan_sig(base, engine_hook::UPDATE_WATER_STATE_SIG)?;
        let trace = engine_hook::trace_global_from(func).read();
        let (slot, original) =
            engine_hook::install_contents_hook(trace, &engine_hook::os::PLATFORM)?;
        let mut hooks = vec![(
            slot,
            original,
            engine_hook::hook_get_point_contents as *const () as usize,
        )];

        // Ray_t layout: where this build keeps m_IsSwept.
        // Linux: the server's Ray_t is the old layout (m_IsRay +0x40, m_IsSwept +0x41).
        #[cfg(not(windows))]
        let ray_note = {
            engine_hook::RAY_SWEPT_OFFSET.store(
                engine_hook::RAY_SWEPT_OFFSET_OLD,
                std::sync::atomic::Ordering::SeqCst,
            );
            format!(
                " (Ray_t m_IsSwept at +{:#x})",
                engine_hook::RAY_SWEPT_OFFSET_OLD
            )
        };
        #[cfg(windows)]
        let ray_note = match engine_hook::scan_sig(base, engine_hook::RAY_HELPER_SIG) {
            Ok(f) => {
                let off = engine_hook::ray_swept_offset_from(f);
                engine_hook::RAY_SWEPT_OFFSET.store(off, std::sync::atomic::Ordering::SeqCst);
                format!(" (Ray_t m_IsSwept at +{off:#x})")
            }
            Err(why) => {
                engine_hook::RAY_SWEPT_OFFSET.store(
                    engine_hook::RAY_SWEPT_OFFSET_OLD,
                    std::sync::atomic::Ordering::SeqCst,
                );
                format!(" (Ray_t layout assumed old: {why})")
            }
        };

        // Second hook: traces. If it fails the point-contents hook stays.
        let trace_hook = engine_hook::hook_trace_ray as *const () as usize;
        let summary = match engine_hook::install_slot_hook(
            trace,
            engine_hook::TRACE_RAY_SLOT,
            trace_hook,
            &engine_hook::ORIGINAL_TRACE_RAY,
            &engine_hook::os::PLATFORM,
        ) {
            Ok((entry, orig)) => {
                hooks.push((entry, orig, trace_hook));
                format!("GetPointContents + TraceRay{ray_note}")
            }
            Err(why) => format!("GetPointContents only (TraceRay hook failed: {why})"),
        };
        *CONTENTS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = hooks;

        // Third hook: bubble trails are clipped to the volume they are in.
        let bubble =
            match engine_hook::scan_sig(base, engine_hook::BUBBLE_TRAIL_SIG).and_then(|t| {
                engine_hook::install_patch(
                    t,
                    engine_hook::BUBBLE_TRAIL_SIG,
                    engine_hook::BUBBLE_TRAIL_STOLEN,
                    engine_hook::hook_bubble_trail as *const () as usize,
                    &engine_hook::os::PLATFORM,
                )
            }) {
                Ok(patch) => {
                    engine_hook::BUBBLE_TRAMPOLINE
                        .store(patch.trampoline, std::sync::atomic::Ordering::SeqCst);
                    *BUBBLE
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(patch);
                    " + UTIL_BubbleTrail".to_string()
                }
                Err(why) => format!(" (bubble-trail clipping inactive: {why})"),
            };
        // Fluids destroyed while props are in them must not crash FluidEndTouch.
        let end_touch = match engine_hook::scan_sig(base, engine_hook::FLUID_END_TOUCH_SIG)
            .and_then(|t| {
                engine_hook::learn_fluid_layout(t)?;
                engine_hook::install_patch(
                    t,
                    engine_hook::FLUID_END_TOUCH_SIG,
                    engine_hook::FLUID_END_TOUCH_STOLEN,
                    engine_hook::hook_fluid_end_touch as *const () as usize,
                    &engine_hook::os::PLATFORM,
                )
            }) {
            Ok(patch) => {
                engine_hook::FLUID_END_TOUCH_TRAMPOLINE
                    .store(patch.trampoline, std::sync::atomic::Ordering::SeqCst);
                *FLUID_END_TOUCH
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(patch);
                " + FluidEndTouch".to_string()
            }
            Err(why) => format!(" (removing a volume with props in it may crash: {why})"),
        };
        if let Ok(t) = engine_hook::scan_sig(base, engine_hook::DISPATCH_EFFECT_SIG) {
            engine_hook::DISPATCH_EFFECT_ADDR
                .store(t as usize, std::sync::atomic::Ordering::SeqCst);
        }

        // Explosion events in a volume become the water explosion effect.
        let te_note = (|| -> Result<(), &'static str> {
            if engine_hook::DISPATCH_EFFECT_ADDR.load(std::sync::atomic::Ordering::SeqCst) == 0 {
                return Err("DispatchEffect not found");
            }
            let vtable = engine_hook::te_explosion_vtable(base)?;
            engine_hook::TE_EXPLOSION_VTABLE.store(vtable, std::sync::atomic::Ordering::SeqCst);
            let create = engine_hook::scan_sig(base, engine_hook::TE_CREATE_SIG)?;
            let patch = engine_hook::install_patch_relocating(
                create,
                engine_hook::TE_CREATE_SIG,
                engine_hook::TE_CREATE_STOLEN,
                engine_hook::TE_CREATE_RIP_LEAS,
                engine_hook::hook_te_create as *const () as usize,
                &engine_hook::os::PLATFORM,
            )?;
            engine_hook::ORIGINAL_TE_CREATE
                .store(patch.trampoline, std::sync::atomic::Ordering::SeqCst);
            *TE_HOOK
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(patch);
            Ok(())
        })();
        let te_note = match te_note {
            Ok(()) => " + explosion events".to_string(),
            Err(why) => format!(" (explosion events not hooked: {why})"),
        };

        // The world entity, named as the hit entity of every water hit. If it
        // cannot be found, water hits that the engine gave no entity are
        // skipped rather than reported without one.
        let world_note = match engine_hook::entity_array_global(base) {
            Ok(g) => {
                engine_hook::ENTITY_ARRAY_GLOBAL.store(g, std::sync::atomic::Ordering::SeqCst);
                String::new()
            }
            Err(why) => format!(" (world entity unknown: {why})"),
        };

        // Fourth: explosives in a volume take the water explosion.
        let mut patches = Vec::new();
        for i in 0..engine_hook::EXPLOSION_TARGETS.len() {
            let r = engine_hook::find_explosion_target(base, i).and_then(|(t, sig, stolen)| {
                engine_hook::install_patch(
                    t,
                    sig,
                    stolen,
                    engine_hook::EXPLOSION_HOOKS[i] as usize,
                    &engine_hook::os::PLATFORM,
                )
            });
            if let Ok(patch) = r {
                engine_hook::EXPLOSION_TRAMPOLINES[i]
                    .store(patch.trampoline, std::sync::atomic::Ordering::SeqCst);
                patches.push(patch);
            }
        }
        // A parenthesis marks a part that is not working (`hook_state::install` callers
        // report those whatever `developer` says).
        let explosion = if patches.len() == engine_hook::EXPLOSION_TARGETS.len() {
            format!(" + {0}/{0} explosion checks", patches.len())
        } else {
            format!(
                " (only {}/{} explosion checks hooked)",
                patches.len(),
                engine_hook::EXPLOSION_TARGETS.len()
            )
        };
        *EXPLOSIONS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = patches;
        Ok(format!(
            "{summary}{bubble}{end_touch}{explosion}{te_note}{world_note}"
        ))
    }
}

#[cfg(feature = "server")]
pub(crate) static EXPLOSIONS: Mutex<Vec<engine_hook::Patch>> = Mutex::new(Vec::new());
#[cfg(feature = "server")]
pub(crate) static TE_HOOK: Mutex<Option<engine_hook::Patch>> = Mutex::new(None);

#[cfg(feature = "server")]
pub(crate) static BUBBLE: Mutex<Option<engine_hook::Patch>> = Mutex::new(None);
#[cfg(feature = "server")]
pub(crate) static FLUID_END_TOUCH: Mutex<Option<engine_hook::Patch>> = Mutex::new(None);

#[cfg(feature = "server")]
/// (id, controller pointer, contents) for every fluid this module created.
pub(crate) static FLUIDS: Mutex<Vec<(u32, usize, c_int)>> = Mutex::new(Vec::new());
#[cfg(feature = "server")]
static NEXT_FLUID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

#[cfg(feature = "server")]
unsafe fn physenv() -> Result<*mut c_void, &'static str> {
    unsafe {
        let base = engine_hook::os::server_base();
        if base.is_null() {
            return Err("the server module is not loaded");
        }
        let at = engine_hook::scan_sig(base, engine_hook::PHYSENV_SIG)
            .map_err(|_| "physics environment pointer not found in server.dll")?;
        let env = engine_hook::physenv_global_from(at).read();
        if env.is_null() {
            return Err("the physics environment has not been created yet");
        }
        Ok(env)
    }
}

#[cfg(feature = "server")]
pub(crate) unsafe fn fluid_attach(
    object: usize,
    plane: [f32; 4],
    fluid: &FluidSettings,
) -> Result<u32, &'static str> {
    unsafe {
        let env = physenv()?;
        let params = engine_hook::FluidParams {
            surface_plane: plane,
            current_velocity: fluid.current,
            damping: fluid.damping,
            torque_factor: fluid.torque,
            viscosity_factor: fluid.viscosity,
            game_data: std::ptr::null_mut(),
            use_aerodynamics: u8::from(fluid.aerodynamics),
            contents: fluid.contents,
        };
        if object == 0 {
            return Err("null physics object");
        }
        engine_hook::add_fluid_callback_flag(object as *mut _);
        let controller = engine_hook::create_fluid(env, object as *mut _, &params);
        if controller.is_null() {
            return Err("the engine refused to create a fluid controller (object has no physics?)");
        }
        let id = NEXT_FLUID.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        FLUIDS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push((id, controller as usize, fluid.contents));
        engine_hook::fluid_born(controller as usize);
        Ok(id)
    }
}

#[cfg(feature = "server")]
pub(crate) unsafe fn fluid_detach(id: u32) -> bool {
    unsafe {
        let taken = {
            let mut list = FLUIDS
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            list.iter().position(|f| f.0 == id).map(|i| list.remove(i))
        };
        let Some((_, fluid, contents)) = taken else {
            return false;
        };
        let Ok(env) = physenv() else {
            return false;
        };
        engine_hook::destroy_fluid(env, fluid as *mut _);
        engine_hook::fluid_died(fluid, contents);
        true
    }
}

#[cfg(feature = "server")]
pub(crate) unsafe fn fluid_detach_all() {
    unsafe {
        let ids: Vec<u32> = FLUIDS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .map(|f| f.0)
            .collect();
        for id in ids {
            fluid_detach(id);
        }
    }
}

#[cfg(feature = "server")]
pub(crate) fn fluid_count() -> usize {
    FLUIDS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .len()
}

pub(crate) unsafe fn uninstall_contents() {
    unsafe {
        engine_hook::RAY_SWEPT_OFFSET.store(
            engine_hook::RAY_SWEPT_OFFSET_OLD,
            std::sync::atomic::Ordering::SeqCst,
        );
        #[cfg(feature = "server")]
        {
            engine_hook::DISPATCH_EFFECT_ADDR.store(0, std::sync::atomic::Ordering::SeqCst);
            let te_patch = TE_HOOK
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            if let Some(patch) = te_patch {
                engine_hook::uninstall_patch(&patch, &engine_hook::os::PLATFORM);
            }
            engine_hook::TE_EXPLOSION_VTABLE.store(0, std::sync::atomic::Ordering::SeqCst);
            let explosion_patches = std::mem::take(
                &mut *EXPLOSIONS
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            );
            for patch in explosion_patches {
                engine_hook::uninstall_patch(&patch, &engine_hook::os::PLATFORM);
            }
            let patch = FLUID_END_TOUCH
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            if let Some(patch) = patch {
                engine_hook::uninstall_patch(&patch, &engine_hook::os::PLATFORM);
            }
            engine_hook::forget_dead_fluids();
            // Bubble detour first: it calls through the trampoline, not the slots.
            let patch = BUBBLE
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            if let Some(patch) = patch {
                engine_hook::uninstall_patch(&patch, &engine_hook::os::PLATFORM);
            }
        }
        let taken = std::mem::take(
            &mut *CONTENTS
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        for (entry, original, hook) in taken.into_iter().rev() {
            engine_hook::uninstall_slot_hook(entry, hook, original, &engine_hook::os::PLATFORM);
        }
    }
}

pub(crate) fn contents_active() -> bool {
    !CONTENTS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .is_empty()
}

#[cfg(all(feature = "client", any(windows, target_os = "linux")))]
/// Client realm: the same two hooks on the client's own `IEngineTrace`, so what the
/// client does with water (ripples and splashes, the underwater sound filter,
/// prediction) sees the volumes the Lua side gave it.
pub(crate) unsafe fn install_client_contents() -> Result<String, &'static str> {
    unsafe {
        let base = engine_hook::os::client_base();
        if base.is_null() {
            return Err("client.dll is not loaded");
        }
        let at = engine_hook::scan_sig(base, engine_hook::CLIENT_TRACE_SIG)?;
        let trace = engine_hook::trace_global_from_load(at).read();
        let (slot, original) =
            engine_hook::install_contents_hook(trace, &engine_hook::os::PLATFORM)?;
        let mut hooks = vec![(
            slot,
            original,
            engine_hook::hook_get_point_contents as *const () as usize,
        )];
        // TraceRay needs the Ray_t layout and a world entity to name in the result;
        // without either only the point contents are hooked.
        let entities = engine_hook::os::client_interface(c"VClientEntityList003");
        // Windows: where this build keeps m_IsSwept comes from the Ray_t helper. The
        // Linux client has the old layout (m_IsRay +0x40, m_IsSwept +0x41).
        #[cfg(windows)]
        let ray = engine_hook::scan_sig(base, engine_hook::RAY_HELPER_SIG);
        #[cfg(not(windows))]
        let ray: Result<*mut u8, &'static str> = Ok(std::ptr::null_mut());
        let note = match (ray, entities) {
            (Ok(f), e) if e != 0 => {
                #[cfg(windows)]
                let off = engine_hook::ray_swept_offset_from(f);
                #[cfg(not(windows))]
                let off = {
                    let _ = f;
                    engine_hook::RAY_SWEPT_OFFSET_OLD
                };
                engine_hook::RAY_SWEPT_OFFSET.store(off, std::sync::atomic::Ordering::SeqCst);
                engine_hook::CLIENT_ENTITY_LIST.store(e, std::sync::atomic::Ordering::SeqCst);
                let trace_hook = engine_hook::hook_trace_ray as *const () as usize;
                match engine_hook::install_slot_hook(
                    trace,
                    engine_hook::TRACE_RAY_SLOT,
                    trace_hook,
                    &engine_hook::ORIGINAL_TRACE_RAY,
                    &engine_hook::os::PLATFORM,
                ) {
                    Ok((entry, orig)) => {
                        hooks.push((entry, orig, trace_hook));
                        format!("GetPointContents + TraceRay (Ray_t m_IsSwept at +{off:#x})")
                    }
                    Err(why) => format!("GetPointContents only (TraceRay hook failed: {why})"),
                }
            }
            (Err(why), _) => format!("GetPointContents only (Ray_t helper: {why})"),
            _ => "GetPointContents only (no VClientEntityList003)".to_string(),
        };
        *CONTENTS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = hooks;
        Ok(note)
    }
}
