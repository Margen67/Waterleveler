// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67

use super::*;
use std::sync::atomic::Ordering;

#[test]
pub(crate) fn te_explosion_vtable_is_found_and_slot_4_is_create() {
    let Some(d) = server_dll() else {
        eprintln!("SERVER_DLL_X64 not set; skipping");
        return;
    };
    let pe = parse_pe(&d);
    let img = map_image(&d, &pe);
    unsafe {
        let vt = te_explosion_vtable(img.as_ptr()).expect("CTEExplosion vtable");
        let slot4 = (vt as *const usize).add(4).read();
        // The mapped image is not relocated: translate from the preferred base.
        let base = img.as_ptr() as usize;
        let nt = img.as_ptr().add(0x3C).cast::<u32>().read_unaligned() as usize;
        let preferred = img
            .as_ptr()
            .add(nt + 24 + 24)
            .cast::<u64>()
            .read_unaligned() as usize;
        let addr = base + (slot4 - preferred);
        // CBaseTempEntity::Create starts with `mov [rsp+8], rbx`.
        let bytes = std::slice::from_raw_parts(addr as *const u8, 5);
        assert_eq!(bytes, &[0x48, 0x89, 0x5C, 0x24, 0x08]);
    }
}

#[test]
pub(crate) fn water_explosion_effect_data_matches_what_the_engine_builds() {
    let d = water_explosion_effect_data([1.0, 2.0, 3.0]);
    assert_eq!(f32::from_le_bytes(d[8..12].try_into().unwrap()), 3.0);
    assert_eq!(u32::from_le_bytes(d[0x30..0x34].try_into().unwrap()), 0x428);
    assert_eq!(f32::from_le_bytes(d[0x38..0x3c].try_into().unwrap()), 128.0);
    assert_eq!(d[0x88], 1);
}

#[test]
pub(crate) fn explosion_in_a_volume_is_redirected_outside_is_not() {
    let _g = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    volumes().clear();
    volumes().push(Volume {
        id: 1,
        min: [0.0; 3],
        max: [100.0; 3],
        contents: Contents::WATER,
    });
    static NAMES: Mutex<Vec<String>> = Mutex::new(Vec::new());
    unsafe extern "win64" fn fake_dispatch(name: *const u8, _data: *const u8) {
        unsafe {
            let n = CStr::from_ptr(name.cast()).to_string_lossy().to_string();
            NAMES.lock().unwrap().push(n);
        }
    }
    DISPATCH_EFFECT_ADDR.store(fake_dispatch as *const () as usize, Ordering::SeqCst);
    let mut te = [0u8; 0x38];
    let put = |te: &mut [u8], p: [f32; 3]| {
        for (i, v) in p.iter().enumerate() {
            te[TE_ORIGIN_OFFSET + i * 4..TE_ORIGIN_OFFSET + i * 4 + 4]
                .copy_from_slice(&v.to_le_bytes());
        }
    };
    put(&mut te, [500.0, 0.0, 0.0]);
    assert!(!unsafe { redirect_explosion(te.as_ptr()) });
    put(&mut te, [50.0, 50.0, 50.0]);
    assert!(unsafe { redirect_explosion(te.as_ptr()) });
    assert_eq!(
        NAMES.lock().unwrap().as_slice(),
        ["WaterSurfaceExplosion".to_string()]
    );
    DISPATCH_EFFECT_ADDR.store(0, Ordering::SeqCst);
    volumes().clear();
}

#[test]
pub(crate) fn redirected_explosion_carries_the_blast_point_in_the_effect_start() {
    let _g = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    volumes().clear();
    volumes().push(Volume {
        id: 1,
        min: [0.0; 3],
        max: [1000.0; 3],
        contents: Contents::WATER,
    });
    static SEEN: Mutex<Vec<[f32; 6]>> = Mutex::new(Vec::new());
    unsafe extern "win64" fn fake_dispatch(_n: *const u8, data: *const u8) {
        unsafe {
            let f = data.cast::<f32>();
            SEEN.lock().unwrap().push([
                f.read_unaligned(),
                f.add(1).read_unaligned(),
                f.add(2).read_unaligned(),
                f.add(3).read_unaligned(),
                f.add(4).read_unaligned(),
                f.add(5).read_unaligned(),
            ]);
        }
    }
    DISPATCH_EFFECT_ADDR.store(fake_dispatch as *const () as usize, Ordering::SeqCst);
    let mut te = [0u8; 0x38];
    for (i, v) in [10.0f32, 20.0, 30.0].iter().enumerate() {
        te[TE_ORIGIN_OFFSET + i * 4..TE_ORIGIN_OFFSET + i * 4 + 4]
            .copy_from_slice(&v.to_le_bytes());
    }
    assert!(unsafe { redirect_explosion(te.as_ptr()) });
    // the effect is centred on the blast point
    assert_eq!(
        SEEN.lock().unwrap().as_slice(),
        [[10.0, 20.0, 30.0, 0.0, 0.0, 0.0]]
    );
    DISPATCH_EFFECT_ADDR.store(0, Ordering::SeqCst);
    volumes().clear();
}

#[test]
pub(crate) fn te_create_signature_is_unique_and_is_the_vtable_slot_4_function() {
    let Some(d) = server_dll() else {
        eprintln!("SERVER_DLL_X64 not set; skipping");
        return;
    };
    let pe = parse_pe(&d);
    let img = map_image(&d, &pe);
    unsafe {
        let at = scan_sig(img.as_ptr(), TE_CREATE_SIG).expect("must match once");
        // and it is the very function the CTEExplosion vtable names in slot 4.
        let vt = te_explosion_vtable(img.as_ptr()).unwrap();
        let slot4 = (vt as *const usize).add(4).read();
        let nt = img.as_ptr().add(0x3C).cast::<u32>().read_unaligned() as usize;
        let preferred = img
            .as_ptr()
            .add(nt + 24 + 24)
            .cast::<u64>()
            .read_unaligned() as usize;
        assert_eq!(img.as_ptr() as usize + (slot4 - preferred), at as usize);
    }
}

#[test]
pub(crate) fn dispatch_effect_signature_is_unique() {
    let Some(d) = server_dll() else {
        eprintln!("SERVER_DLL_X64 not set; skipping");
        return;
    };
    let pe = parse_pe(&d);
    let img = map_image(&d, &pe);
    unsafe {
        let at = scan_sig(img.as_ptr(), DISPATCH_EFFECT_SIG).expect("must match once");
        let _ = at;
    }
}

#[test]
pub(crate) fn explosion_signatures_are_unique() {
    let Some(d) = server_dll() else {
        eprintln!("SERVER_DLL_X64 not set; skipping");
        return;
    };
    let pe = parse_pe(&d);
    let img = map_image(&d, &pe);
    for i in 0..EXPLOSION_TARGETS.len() {
        unsafe {
            find_explosion_target(img.as_ptr(), i)
                .expect("explosion signature must match exactly once");
        }
    }
}

#[test]
pub(crate) fn explosion_hook_raises_water_level_only_inside_a_volume() {
    let _g = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut ent = vec![0u8; 0x400];
    let set = |ent: &mut Vec<u8>, p: [f32; 3]| {
        for i in 0..3 {
            ent[ENTITY_ORIGIN_OFFSET + i * 4..ENTITY_ORIGIN_OFFSET + i * 4 + 4]
                .copy_from_slice(&p[i].to_le_bytes());
        }
    };
    volumes().clear();
    volumes().push(Volume {
        id: 1,
        min: [0.0; 3],
        max: [100.0; 3],
        contents: Contents::WATER,
    });
    set(&mut ent, [500.0, 0.0, 0.0]);
    unsafe { raise_water_level_in_volume(ent.as_mut_ptr()) };
    assert_eq!(ent[ENTITY_WATER_LEVEL_OFFSET], 0);
    set(&mut ent, [50.0, 50.0, 50.0]);
    unsafe { raise_water_level_in_volume(ent.as_mut_ptr()) };
    assert_eq!(ent[ENTITY_WATER_LEVEL_OFFSET], 2);
    ent[ENTITY_WATER_LEVEL_OFFSET] = 3;
    unsafe { raise_water_level_in_volume(ent.as_mut_ptr()) };
    assert_eq!(ent[ENTITY_WATER_LEVEL_OFFSET], 3);
    volumes().clear();
}
