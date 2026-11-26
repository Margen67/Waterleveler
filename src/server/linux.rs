// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67

use crate::*;
use std::sync::atomic::Ordering;

pub(crate) unsafe fn entity_index(this: *const u8) -> Option<i32> {
    unsafe {
        let handle = this
            .add(ENTITY_HANDLE_OFFSET)
            .cast::<u32>()
            .read_unaligned();
        (handle != u32::MAX).then_some((handle & 0x3fff) as i32)
    }
}

pub(crate) const ENTITY_HANDLE_OFFSET: usize = 8;

/// Linux `CBaseEntity::SetWaterLevel`: `m_nWaterLevel` is at +0x2d3 (also a layout check).
pub(crate) const SIG: &str = linux_sigs::SIG;

/// Linux `CBaseEntity::UpdateWaterState`: the `enginetrace` variable is reached with the
/// last `lea r13,[rip+disp32]` in the match, then `mov rdi,[r13]; mov rax,[rdi]; call [rax]`.
pub(crate) const UPDATE_WATER_STATE_SIG: &str = linux_sigs::UPDATE_WATER_STATE_SIG;

pub(crate) const ENTITY_ARRAY_SIG: &str = linux_sigs::ENTITY_ARRAY_SIG;

pub(crate) const PHYSENV_SIG: &str = linux_sigs::PHYSENV_SIG;

/// Linux vtables have two destructor slots, so the later slots are one higher.
pub(crate) const CREATE_FLUID_SLOT: usize = 12;

pub(crate) const DESTROY_FLUID_SLOT: usize = 13;

pub(crate) const OBJ_SET_CALLBACK_FLAGS_SLOT: usize = 0xb8 / 8;

pub(crate) const OBJ_GET_CALLBACK_FLAGS_SLOT: usize = 0xc0 / 8;

pub(crate) const FLUID_END_TOUCH_SIG: &str = linux_sigs::FLUID_END_TOUCH_SIG;

pub(crate) const OBJ_GET_GAME_DATA_SLOT: usize = 0x90 / 8;

pub(crate) const ENTITY_EFLAGS_OFFSET: usize = 0x1b8;

pub(crate) const ENTITY_EVENT_SLOT: usize = 0x2f0 / 8;

pub(crate) const BUBBLE_TRAIL_SIG: &str = linux_sigs::BUBBLE_TRAIL_SIG;

pub(crate) const EXPLOSION_TARGETS: [&[&str]; 3] = linux_sigs::EXPLOSION_TARGETS;

pub(crate) const ENTITY_WATER_LEVEL_OFFSET: usize = 0x2d3;

pub(crate) const ENTITY_ORIGIN_OFFSET: usize = 0x364;

#[allow(clippy::missing_const_for_fn, clippy::unnecessary_wraps)]
pub(crate) unsafe fn learn_water_level_layout(_at: *const u8) -> Result<usize, &'static str> {
    Ok(ENTITY_WATER_LEVEL_OFFSET)
}

#[allow(clippy::missing_const_for_fn, clippy::unnecessary_wraps)]
pub(crate) unsafe fn learn_fluid_layout(_at: *const u8) -> Result<(), &'static str> {
    Ok(())
}

pub(crate) const DISPATCH_EFFECT_SIG: &str = linux_sigs::DISPATCH_EFFECT_SIG;

pub(crate) const TE_SCALE_OFFSET: usize = 0x24;

pub(crate) const TE_FLAGS_OFFSET: usize = 0x28;

pub(crate) const TE_MAGNITUDE_OFFSET: usize = 0x30;

/// `CBaseTempEntity::m_pszName` (a `const char*`).
pub(crate) const TE_NAME_OFFSET: usize = 8;

pub(crate) unsafe fn is_te_explosion(this: *const u8) -> bool {
    unsafe {
        if TE_EXPLOSION_VTABLE.load(Ordering::Relaxed) == 0 {
            return false;
        }
        let name = this
            .add(TE_NAME_OFFSET)
            .cast::<*const u8>()
            .read_unaligned();
        !name.is_null() && CStr::from_ptr(name.cast()).to_bytes() == b"Explosion"
    }
}

/// The Linux server has no RTTI for `CTEExplosion`, so no vtable is looked up: the hook
/// recognises the object by its name instead ([`is_te_explosion`]). The returned `1` is
/// only the "armed" marker stored in [`TE_EXPLOSION_VTABLE`].
#[allow(clippy::unnecessary_wraps)] // same signature as the Windows lookup, which can fail
pub(crate) const unsafe fn te_explosion_vtable(_base: *const u8) -> Result<usize, &'static str> {
    Ok(1)
}

pub(crate) const TE_CREATE_SIG: &str = linux_sigs::TE_CREATE_SIG;
