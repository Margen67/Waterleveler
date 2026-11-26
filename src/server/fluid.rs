// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67

use crate::*;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Address of the engine's `physenv` variable.
pub(crate) unsafe fn physenv_global_from(
    func: *const u8,
) -> Result<*const *mut c_void, &'static str> {
    unsafe {
        let code = decode_match(func, PHYSENV_SIG);
        only_global(&code).map(|global| global as *const *mut c_void)
    }
}

/// `fluidparams_t` (x64 layout).
#[repr(C)]
pub(crate) struct FluidParams {
    pub surface_plane: [f32; 4],
    pub current_velocity: [f32; 3],
    pub damping: f32,
    pub torque_factor: f32,
    pub viscosity_factor: f32,
    pub game_data: *mut c_void,
    pub use_aerodynamics: u8,
    pub contents: i32,
}

pub(crate) type CreateFluidFn =
    engine_fn!((*mut c_void, *mut c_void, *const FluidParams) -> *mut c_void);

pub(crate) type DestroyFluidFn = engine_fn!((*mut c_void, *mut c_void));

pub(crate) unsafe fn create_fluid(
    env: *mut c_void,
    object: *mut c_void,
    params: &FluidParams,
) -> *mut c_void {
    unsafe {
        let vtable = (env as *const *const usize).read();
        let f: CreateFluidFn = std::mem::transmute(vtable.add(CREATE_FLUID_SLOT).read());
        f(env, object, params)
    }
}

/// The engine ORs this into the world water object before creating its
/// fluid controller (`CALLBACK_GLOBAL_TOUCH_STATIC`, unconfirmed name).
pub(crate) const FLUID_OBJECT_CALLBACK_FLAG: u16 = 0x200;

pub(crate) type GetFlagsFn = engine_fn!((*mut c_void) -> u16);

pub(crate) type SetFlagsFn = engine_fn!((*mut c_void, u16));

/// Mirrors the engine's setup of a water object. Returns the old flags.
pub(crate) unsafe fn add_fluid_callback_flag(object: *mut c_void) -> u16 {
    unsafe {
        let vtable = (object as *const *const usize).read();
        let get: GetFlagsFn = std::mem::transmute(vtable.add(OBJ_GET_CALLBACK_FLAGS_SLOT).read());
        let set: SetFlagsFn = std::mem::transmute(vtable.add(OBJ_SET_CALLBACK_FLAGS_SLOT).read());
        let old = get(object);
        set(object, old | FLUID_OBJECT_CALLBACK_FLAG);
        old
    }
}

pub(crate) unsafe fn destroy_fluid(env: *mut c_void, fluid: *mut c_void) {
    unsafe {
        let vtable = (env as *const *const usize).read();
        let f: DestroyFluidFn = std::mem::transmute(vtable.add(DESTROY_FLUID_SLOT).read());
        f(env, fluid);
    }
}

pub(crate) static FLUID_END_TOUCH_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);

bitflags::bitflags! {
    /// `CBaseEntity::m_iEFlags` bits this module touches; the rest are kept as they are.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) struct EFlags: u32 {
        const TOUCHING_FLUID = 0x80000;
    }
}

pub(crate) const ENTITY_EVENT_WATER_UNTOUCH: c_int = 1;

/// Controllers destroyed by this module (address, contents) that
/// vphysics objects may still point at.
pub(crate) static DEAD_FLUIDS: Mutex<Vec<(usize, c_int)>> = Mutex::new(Vec::new());

pub(crate) const DEAD_FLUIDS_MAX: usize = 256;

pub(crate) fn fluid_died(controller: usize, contents: c_int) {
    let mut dead = DEAD_FLUIDS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    dead.retain(|d| d.0 != controller);
    if dead.len() >= DEAD_FLUIDS_MAX {
        dead.remove(0);
    }
    dead.push((controller, contents));
}

/// A new controller may reuse a dead one's address.
pub(crate) fn fluid_born(controller: usize) {
    DEAD_FLUIDS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .retain(|d| d.0 != controller);
}

pub(crate) fn forget_dead_fluids() {
    DEAD_FLUIDS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clear();
}

engine_abi_item! {
pub(crate) unsafe fn hook_fluid_end_touch(
    this: *mut c_void,
    object: *mut c_void,
    fluid: *mut c_void,
) {
    unsafe {
        // Never block inside an engine callback.
        let dead = DEAD_FLUIDS
            .try_lock()
            .ok()
            .and_then(|d| d.iter().find(|e| e.0 == fluid as usize).map(|e| e.1));
        let Some(contents) = dead else {
            let original: engine_fn!((*mut c_void, *mut c_void, *mut c_void)) =
                std::mem::transmute(FLUID_END_TOUCH_TRAMPOLINE.load(Ordering::Relaxed));
            original(this, object, fluid);
            return;
        };
        if object.is_null() {
            return;
        }
        let get_game_data: engine_fn!((*mut c_void) -> *mut u8) =
            std::mem::transmute(
                *(*(object as *const *const usize)).add(OBJ_GET_GAME_DATA_SLOT),
            );
        let entity = get_game_data(object);
        if entity.is_null() {
            return;
        }
        let flags = entity.add(EFLAGS_OFFSET.load(Ordering::Relaxed)).cast::<u32>();
        flags.write_unaligned(
            EFlags::from_bits_retain(flags.read_unaligned())
                .difference(EFlags::TOUCHING_FLUID)
                .bits(),
        );
        let on_event: engine_fn!((*mut u8, c_int, *mut c_void)) =
            std::mem::transmute(
                *(*(entity as *const *const usize)).add(EVENT_SLOT.load(Ordering::Relaxed)),
            );
        on_event(
            entity,
            ENTITY_EVENT_WATER_UNTOUCH,
            contents as isize as *mut c_void,
        );
    }
}
}
