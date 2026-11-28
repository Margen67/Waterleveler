// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67

use crate::*;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

/// The first alternative of target `i` that matches exactly once.
pub(crate) unsafe fn find_explosion_target(
    base: *const u8,
    i: usize,
) -> Result<*mut u8, &'static str> {
    unsafe {
        let mut last = "signature not found (different server.dll build?)";
        for sig in EXPLOSION_TARGETS[i] {
            match scan_sig(base, sig) {
                Ok(at) => return Ok(at),
                Err(e) => last = e,
            }
        }
        Err(last)
    }
}

pub(crate) static EXPLOSION_TRAMPOLINES: [AtomicUsize; 3] = [
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
];

pub(crate) static EXPLOSION_FIXES: AtomicU64 = AtomicU64::new(0);

pub(crate) type ExplosionFn = engine_fn!((*mut u8, usize, usize, usize) -> usize);

pub(crate) unsafe fn explosion_hook(
    i: usize,
    this: *mut u8,
    a: usize,
    b: usize,
    c: usize,
) -> usize {
    unsafe {
        raise_water_level_in_volume(this);
        let original: ExplosionFn =
            std::mem::transmute(EXPLOSION_TRAMPOLINES[i].load(Ordering::Relaxed));
        original(this, a, b, c)
    }
}

engine_abi_item! {
pub(crate) unsafe fn hook_explosion0(
    t: *mut u8,
    a: usize,
    b: usize,
    c: usize,
) -> usize {
    unsafe { explosion_hook(0, t, a, b, c) }
}
}

engine_abi_item! {
pub(crate) unsafe fn hook_explosion1(
    t: *mut u8,
    a: usize,
    b: usize,
    c: usize,
) -> usize {
    unsafe { explosion_hook(1, t, a, b, c) }
}
}

engine_abi_item! {
pub(crate) unsafe fn hook_explosion2(
    t: *mut u8,
    a: usize,
    b: usize,
    c: usize,
) -> usize {
    unsafe { explosion_hook(2, t, a, b, c) }
}
}

pub(crate) const EXPLOSION_HOOKS: [ExplosionFn; 3] =
    [hook_explosion0, hook_explosion1, hook_explosion2];

// ---- Explosion events (CTEExplosion::Create) -------------------------
//
// Most explosives (barrels, RPG, grenades, SLAM ...) send the client an
// explosion *event*, and the client decides for itself, with its own
// point-contents check, whether to play the normal explosion or the water
// one (underwater sound, splash, wake). A volume does not exist for that
// check. The one thing the server can do is what the engine's own water
// explosion code does: send the "WaterSurfaceExplosion" effect instead.
// So CTEExplosion's Create (vtable slot 4) is swapped: an explosion whose
// origin (this+0x18) is in a volume is sent as that effect, and the event
// is dropped.

pub(crate) const TE_ORIGIN_OFFSET: usize = 0x18;

pub(crate) static ORIGINAL_TE_CREATE: AtomicUsize = AtomicUsize::new(0);

pub(crate) static DISPATCH_EFFECT_ADDR: AtomicUsize = AtomicUsize::new(0);

pub(crate) static EXPLOSIONS_REDIRECTED: AtomicU64 = AtomicU64::new(0);

/// Fills a `CEffectData` the way the engine does for the water explosion
/// (flags 0x428, scale 128, magnitude 50; byte at +0x88 set).
#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
pub(crate) fn water_explosion_effect_data(origin: [f32; 3]) -> [u8; 0xa0] {
    water_explosion_effect_data_with(origin, 0x428, 128.0, 50.0)
}

/// Same layout the client's own in-water path hands to the water explosion
/// (flags, scale and magnitude are the temp entity's). The client plays
/// WaterExplosionEffect.Sound only when scale > 128, else the base sound.
pub(crate) fn water_explosion_effect_data_with(
    origin: [f32; 3],
    flags: u32,
    scale: f32,
    magnitude: f32,
) -> [u8; 0xa0] {
    zerocopy::transmute!(EffectData::new(origin, flags, scale, magnitude))
}

/// True (and the effect sent) if the explosion at `this`' origin is in a volume.
pub(crate) unsafe fn redirect_explosion(this: *const u8) -> bool {
    unsafe {
        let dispatch = DISPATCH_EFFECT_ADDR.load(Ordering::Relaxed);
        if this.is_null() || dispatch == 0 {
            return false;
        }
        let origin = read_vec3(this.add(TE_ORIGIN_OFFSET).cast::<f32>());
        let inside = snapshot_with(|v| v.may_contain(origin), |v| v.surface_z(origin).is_some())
            .unwrap_or(false);
        if !inside {
            return false;
        }
        // CTEExplosion fields (see TE_SCALE_OFFSET).
        // The effect gets the blast point: the client traces up from it to find
        // the surface and the depth (which picks the muffled sound).
        let scale = this.add(TE_SCALE_OFFSET).cast::<f32>().read_unaligned();
        let flags = this.add(TE_FLAGS_OFFSET).cast::<u32>().read_unaligned();
        let magnitude = this.add(TE_MAGNITUDE_OFFSET).cast::<i32>().read_unaligned() as f32;
        let data = water_explosion_effect_data_with(origin, flags, scale, magnitude);
        let f: engine_fn!((*const u8, *const u8)) = std::mem::transmute(dispatch);
        f(c"WaterSurfaceExplosion".as_ptr().cast(), data.as_ptr());
        bump(&EXPLOSIONS_REDIRECTED);
        true
    }
}

pub(crate) static TE_EXPLOSION_VTABLE: AtomicUsize = AtomicUsize::new(0);

engine_abi_item! {
pub(crate) unsafe fn hook_te_create(
    this: *mut u8,
    filter: *mut c_void,
    delay: f32,
) {
    unsafe {
        if !this.is_null() && is_te_explosion(this) && redirect_explosion(this) {
            return;
        }
        let original: engine_fn!((*mut u8, *mut c_void, f32)) =
            std::mem::transmute(ORIGINAL_TE_CREATE.load(Ordering::Relaxed));
        original(this, filter, delay);
    }
}
}
