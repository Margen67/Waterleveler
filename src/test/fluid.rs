// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67

use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

pub(crate) static END_TOUCH_ORIGINAL_CALLS: AtomicUsize = AtomicUsize::new(0);

pub(crate) static UNTOUCH_EVENTS: AtomicUsize = AtomicUsize::new(0);

pub(crate) static UNTOUCH_CONTENTS: AtomicUsize = AtomicUsize::new(0);

pub(crate) unsafe extern "win64" fn fake_end_touch(_: *mut c_void, _: *mut c_void, _: *mut c_void) {
    END_TOUCH_ORIGINAL_CALLS.fetch_add(1, Ordering::SeqCst);
}

#[allow(clippy::missing_const_for_fn)] // a stand-in for an engine function
pub(crate) unsafe extern "win64" fn fake_get_game_data(object: *mut c_void) -> *mut u8 {
    unsafe { *(object as *const *mut u8).add(1) }
}

pub(crate) unsafe extern "win64" fn fake_on_entity_event(
    _: *mut u8,
    event: c_int,
    data: *mut c_void,
) {
    assert_eq!(event, ENTITY_EVENT_WATER_UNTOUCH);
    UNTOUCH_EVENTS.fetch_add(1, Ordering::SeqCst);
    UNTOUCH_CONTENTS.store(data as usize, Ordering::SeqCst);
}

#[test]
pub(crate) fn end_touch_with_a_destroyed_fluid_skips_the_splash_code() {
    let _g = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    FLUID_END_TOUCH_TRAMPOLINE.store(fake_end_touch as *const () as usize, Ordering::SeqCst);
    END_TOUCH_ORIGINAL_CALLS.store(0, Ordering::SeqCst);
    UNTOUCH_EVENTS.store(0, Ordering::SeqCst);
    // A fake entity (vtable + eflags) and a fake physics object that returns it.
    let mut entity_vtable = vec![0usize; ENTITY_EVENT_SLOT + 1];
    entity_vtable[ENTITY_EVENT_SLOT] = fake_on_entity_event as *const () as usize;
    let mut entity = vec![0u8; 0x400];
    entity[..8].copy_from_slice(&(entity_vtable.as_ptr() as usize).to_le_bytes());
    entity[ENTITY_EFLAGS_OFFSET..ENTITY_EFLAGS_OFFSET + 4]
        .copy_from_slice(&(EFlags::TOUCHING_FLUID.bits() | 0x4).to_le_bytes());
    let mut object_vtable = [0usize; OBJ_GET_GAME_DATA_SLOT + 1];
    object_vtable[OBJ_GET_GAME_DATA_SLOT] = fake_get_game_data as *const () as usize;
    let object = [
        object_vtable.as_ptr() as usize,
        entity.as_mut_ptr() as usize,
    ];
    let live = 0x1000usize;
    let dead = 0x2000usize;
    fluid_died(dead, Contents::SLIME.bits());
    unsafe {
        // A live fluid goes to the engine's own code.
        hook_fluid_end_touch(
            std::ptr::null_mut(),
            object.as_ptr().cast_mut().cast::<c_void>(),
            live as *mut c_void,
        );
        assert_eq!(END_TOUCH_ORIGINAL_CALLS.load(Ordering::SeqCst), 1);
        assert_eq!(UNTOUCH_EVENTS.load(Ordering::SeqCst), 0);
        // A destroyed one never reaches it; the entity is released.
        hook_fluid_end_touch(
            std::ptr::null_mut(),
            object.as_ptr().cast_mut().cast::<c_void>(),
            dead as *mut c_void,
        );
    }
    assert_eq!(END_TOUCH_ORIGINAL_CALLS.load(Ordering::SeqCst), 1);
    assert_eq!(UNTOUCH_EVENTS.load(Ordering::SeqCst), 1);
    assert_eq!(
        UNTOUCH_CONTENTS.load(Ordering::SeqCst),
        Contents::SLIME.bits() as usize
    );
    let flags = u32::from_le_bytes(
        entity[ENTITY_EFLAGS_OFFSET..ENTITY_EFLAGS_OFFSET + 4]
            .try_into()
            .unwrap(),
    );
    assert_eq!(flags, 0x4, "only EFlags::TOUCHING_FLUID is cleared");
    // A new controller at the same address is live again.
    fluid_born(dead);
    unsafe {
        hook_fluid_end_touch(
            std::ptr::null_mut(),
            object.as_ptr().cast_mut().cast::<c_void>(),
            dead as *mut c_void,
        );
    }
    assert_eq!(END_TOUCH_ORIGINAL_CALLS.load(Ordering::SeqCst), 2);
    forget_dead_fluids();
}

#[test]
pub(crate) fn fluid_end_touch_signature_is_unique_in_server_dll() {
    let Some(d) = server_dll() else {
        eprintln!("SERVER_DLL_X64 not set; skipping");
        return;
    };
    let pat = parse_sig(FLUID_END_TOUCH_SIG);
    let pe = parse_pe(&d);
    let (_, _, _, text_raw, text_size, _) =
        pe.sections.iter().find(|s| s.0 == ".text").unwrap().clone();
    let hits = find_all(&d[text_raw..text_raw + text_size], &pat);
    assert_eq!(hits.len(), 1, "must match exactly once");
}

#[test]
pub(crate) fn fluid_params_layout_matches_the_engine() {
    use std::mem::{offset_of, size_of};
    assert_eq!(offset_of!(FluidParams, surface_plane), 0x00);
    assert_eq!(offset_of!(FluidParams, current_velocity), 0x10);
    assert_eq!(offset_of!(FluidParams, damping), 0x1c); // vphysics reads [params+0x1c]
    assert_eq!(offset_of!(FluidParams, game_data), 0x28); // [params+0x28]
    assert_eq!(offset_of!(FluidParams, contents), 0x34); // [params+0x34]
    assert_eq!(size_of::<FluidParams>(), 0x38);
}

#[test]
pub(crate) fn physenv_signature_is_unique_and_points_into_data() {
    let Some(d) = server_dll() else {
        eprintln!("SERVER_DLL_X64 not set; skipping");
        return;
    };
    let pe = parse_pe(&d);
    let img = map_image(&d, &pe);
    unsafe {
        let at = scan_sig(img.as_ptr(), PHYSENV_SIG).expect("signature must match exactly once");
        let global = physenv_global_from(at).expect("a load of physenv");
        let grva = global as usize - img.as_ptr() as usize;
        let sec = pe
            .sections
            .iter()
            .find(|s| s.1 <= grva && grva < s.1 + s.2)
            .expect("physenv global outside every section");
        assert!(
            sec.0 == ".data" || sec.0 == ".bss",
            "physenv is in {}",
            sec.0
        );
    }
}

pub(crate) type FluidCall = (&'static str, usize, usize, [f32; 4], f32, i32);

pub(crate) static FLUID_CALLS: Mutex<Vec<FluidCall>> = Mutex::new(Vec::new());

pub(crate) unsafe extern "win64" fn fake_create(
    env: *mut c_void,
    obj: *mut c_void,
    p: *const FluidParams,
) -> *mut c_void {
    unsafe {
        let p = &*p;
        FLUID_CALLS.lock().unwrap().push((
            "create",
            env as usize,
            obj as usize,
            p.surface_plane,
            p.damping,
            p.contents,
        ));
        0x7777 as *mut c_void
    }
}

pub(crate) unsafe extern "win64" fn fake_destroy(env: *mut c_void, fluid: *mut c_void) {
    FLUID_CALLS
        .lock()
        .unwrap()
        .push(("destroy", env as usize, fluid as usize, [0.0; 4], 0.0, 0));
}

pub(crate) static FLAGS: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(0x0001);

pub(crate) unsafe extern "win64" fn fake_get_flags(_: *mut c_void) -> u16 {
    FLAGS.load(Ordering::SeqCst)
}

pub(crate) unsafe extern "win64" fn fake_set_flags(_: *mut c_void, f: u16) {
    FLAGS.store(f, Ordering::SeqCst);
}

#[test]
pub(crate) fn slime_volumes_report_slime_and_only_water_is_a_surface() {
    let vols = [
        Volume {
            id: 1,
            min: [0.0; 3],
            max: [100.0; 3],
            contents: Contents::SLIME,
        },
        Volume {
            id: 2,
            min: [200.0, 0.0, 0.0],
            max: [300.0, 100.0, 100.0],
            contents: Contents::WATER,
        },
    ];
    assert_eq!(contents_at(&vols, [50.0; 3]), Contents::SLIME);
    assert_eq!(contents_at(&vols, [250.0, 50.0, 50.0]), Contents::WATER);
    assert_eq!(contents_at(&vols, [150.0; 3]), Contents::empty());
    assert_eq!(surface_z_at(&vols, [50.0; 3]), None);
    assert_eq!(surface_z_at(&vols, [250.0, 50.0, 50.0]), Some(100.0));
    let both = [
        vols[0],
        Volume { id: 3, ..vols[0] },
        Volume {
            id: 4,
            contents: Contents::WATER,
            ..vols[0]
        },
    ];
    assert_eq!(
        contents_at(&both, [50.0; 3]),
        Contents::SLIME | Contents::WATER
    );
}

#[test]
pub(crate) fn fluid_defaults_match_a_map_water_brush() {
    let d = FluidSettings::default();
    assert_eq!(
        (d.torque, d.viscosity, d.aerodynamics, d.current),
        (0.0, 0.0, false, [0.0; 3])
    );
    assert_eq!(d.damping, 0.01);
    assert_eq!(d.contents, Contents::WATER);
}

#[test]
pub(crate) fn fluid_object_gets_callback_flag_via_slots_0xb0_0xb8() {
    let _g = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    unsafe {
        let mut vtable = [0usize; 32];
        vtable[OBJ_GET_CALLBACK_FLAGS_SLOT] = fake_get_flags as *const () as usize;
        vtable[OBJ_SET_CALLBACK_FLAGS_SLOT] = fake_set_flags as *const () as usize;
        let obj: [*const usize; 1] = [vtable.as_ptr()];
        let old = add_fluid_callback_flag(obj.as_ptr().cast_mut().cast::<c_void>());
        assert_eq!(old, 0x0001);
        assert_eq!(FLAGS.load(Ordering::SeqCst), 0x0201);
    }
}

#[test]
pub(crate) fn fluid_calls_use_vtable_slots_11_and_12() {
    let _g = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    unsafe {
        let mut vtable = [0usize; 16];
        vtable[CREATE_FLUID_SLOT] = fake_create as *const () as usize;
        vtable[DESTROY_FLUID_SLOT] = fake_destroy as *const () as usize;
        let env_obj: [*const usize; 1] = [vtable.as_ptr()];
        let env = env_obj.as_ptr().cast_mut().cast::<c_void>();
        FLUID_CALLS.lock().unwrap().clear();
        let params = FluidParams {
            surface_plane: [0.0, 0.0, 1.0, 192.0],
            current_velocity: [0.0; 3],
            damping: 0.01,
            torque_factor: 0.05,
            viscosity_factor: 0.0,
            game_data: std::ptr::null_mut(),
            use_aerodynamics: 0,
            contents: Contents::WATER.bits(),
        };
        let fluid = create_fluid(env, 0x1234 as *mut c_void, &params);
        assert_eq!(fluid as usize, 0x7777);
        destroy_fluid(env, fluid);
        let calls = FLUID_CALLS.lock().unwrap();
        assert_eq!(
            calls[0],
            (
                "create",
                env as usize,
                0x1234,
                [0.0, 0.0, 1.0, 192.0],
                0.01,
                Contents::WATER.bits()
            )
        );
        assert_eq!(calls[1].0, "destroy");
        assert_eq!(calls[1].2, 0x7777);
        drop(calls);
    }
}
