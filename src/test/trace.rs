// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67

use super::*;
use std::sync::atomic::Ordering;

#[test]
pub(crate) fn client_trace_global_is_read_from_the_load_instruction() {
    // mov rcx,[rip+0x100]; the variable is at (instruction + 7 + 0x100).
    let mut code = vec![0x48u8, 0x8B, 0x0D, 0x00, 0x01, 0x00, 0x00, 0x90];
    // The decoder reads as many bytes as the signature has.
    code.extend_from_slice(&[0; 32]);
    let at = code.as_ptr();
    let g = unsafe { trace_global_from_load(at) }.expect("a load of a global");
    assert_eq!(g as usize, at as usize + 7 + 0x100);
}

#[test]
pub(crate) fn hook_substitutes_override_on_real_function_bytes() {
    let Some(d) = server_dll() else {
        eprintln!("SERVER_DLL_X64 not set; skipping");
        return;
    };
    let pe = parse_pe(&d);
    let (_, text_rva, _, text_raw, _, _) =
        pe.sections.iter().find(|s| s.0 == ".text").unwrap().clone();
    let func = &d[text_raw + (sig_rva(&d, &pe) - text_rva)..][..FUNC_LEN];

    unsafe {
        // The genuine SetWaterLevel machine code, in executable memory.
        let target = rwx(4096);
        std::ptr::copy_nonoverlapping(func.as_ptr(), target, FUNC_LEN);

        // Where this build keeps m_nWaterLevel (0x2ab or 0x2b3), from the function itself;
        // the four places that name it agree.
        let wl = water_level_offset(&decode_match(target, SIG)).expect("water level offset");
        assert!(wl == 0x2ab || wl == 0x2b3, "unexpected offset {wl:#x}");
        let detour = install_detour(target, hook_set_water_level as *const (), &TRAMPOLINE)
            .expect("install failed");
        // The entry now starts with a jump (a relative one when the trampoline is in reach).
        assert!(matches!(*target, 0xE9 | 0xFF));
        assert_ne!(std::slice::from_raw_parts(target, 5), &func[..5]);

        // Fake CBaseEntity: edict index 5, current m_nWaterLevel = 1.
        let mut edict = [0u8; 32];
        edict[6..8].copy_from_slice(&5i16.to_le_bytes());
        let mut entity = vec![0u8; 0x400];
        entity[OFF_PEV..OFF_PEV + 8].copy_from_slice(&(edict.as_ptr() as u64).to_le_bytes());
        entity[wl] = 1;
        let this = entity.as_mut_ptr().cast::<c_void>();

        let set_water_level: unsafe extern "win64" fn(*mut c_void, c_int) =
            std::mem::transmute(target);
        let hits_before = HITS.load(Ordering::Relaxed);

        // 1) Override 1 for entity 5; the engine asks for 3. The hook must
        //    substitute 1, which equals the current value, so the original
        //    code takes its "nothing changed" path (no vtable call, which
        //    would crash on this fake object) and the byte stays 1.
        override_set(5, 1);
        set_water_level(this, 3);
        assert_eq!(entity[wl], 1);
        assert_eq!(HITS.load(Ordering::Relaxed), hits_before + 1);

        // 2) No override: the call goes through the trampoline, which runs
        //    the relocated prologue and continues in the real function.
        //    level == current again so it returns without side effects.
        override_remove(5);
        set_water_level(this, 1);
        assert_eq!(entity[wl], 1);
        assert_eq!(HITS.load(Ordering::Relaxed), hits_before + 1);

        // 3) Entities without a pev are passed through untouched.
        let mut bare = vec![0u8; 0x400];
        bare[wl] = 2;
        set_water_level(bare.as_mut_ptr().cast::<c_void>(), 2);
        assert_eq!(bare[wl], 2);

        // 4) Uninstall restores the original bytes exactly.
        drop(detour);
        assert_eq!(std::slice::from_raw_parts(target, FUNC_LEN), func);
    }
}

#[test]
pub(crate) fn volume_math() {
    let v = [water(1, [0.0, 0.0, 0.0], [128.0, 128.0, 128.0])];
    assert!(point_in_volumes(&v, [64.0, 64.0, 64.0]));
    assert!(point_in_volumes(&v, [0.0, 128.0, 0.0])); // edges are inside
    assert!(!point_in_volumes(&v, [129.0, 64.0, 64.0]));
    assert!(!point_in_volumes(&v, [64.0, 64.0, -0.5]));
    assert!(!point_in_volumes(&[], [0.0, 0.0, 0.0]));
}

#[test]
pub(crate) fn enginetrace_pointer_resolves_in_server_dll() {
    let Some(d) = server_dll() else { return };
    let pe = parse_pe(&d);
    let img = map_image(&d, &pe);
    unsafe {
        let func = scan_sig(img.as_ptr(), UPDATE_WATER_STATE_SIG).expect("scan failed");
        // RVAs move between game updates; only the structure is checked.
        let global = trace_global_from(func).expect("a load of enginetrace");
        let grva = global as usize - img.as_ptr() as usize;
        // The `enginetrace` variable lives in a non-executable section.
        let sec = pe
            .sections
            .iter()
            .find(|s| s.1 <= grva && grva < s.1 + s.2)
            .expect("global is outside every section");
        assert!(
            sec.0 == ".data" || sec.0 == ".bss",
            "enginetrace global is in {}",
            sec.0
        );
    }
}

pub(crate) static GPC_ARGS: Mutex<(usize, usize)> = Mutex::new((0, 0));

/// Stand-in for the engine's `GetPointContents`: reports `Contents::SOLID`.
pub(crate) unsafe extern "win64" fn fake_get_point_contents(
    _this: *mut c_void,
    _pos: *const f32,
    a3: usize,
    a4: usize,
) -> c_int {
    *GPC_ARGS.lock().unwrap() = (a3, a4);
    0x1
}

/// Tests that touch the shared VOLUMES / hook statics must not interleave.
pub(crate) static TEST_LOCK: Mutex<()> = Mutex::new(());

#[test]
pub(crate) fn rays_starting_inside_behave_like_a_real_water_brush() {
    let _g = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let vols = cube();

    // The engine's water-height probe: starts 256 above a point that is
    // inside, ends at that point (also inside) -> allsolid, fraction 0,
    // end point = start.
    let ray = line([64.0, 64.0, 100.0], [0.0, 0.0, -40.0]);
    let mut tr = miss_trace();
    assert!(apply(&vols, &ray, &mut tr, 0x1234));
    assert_eq!(f32at(&tr, 0x2c), 0.0, "fraction");
    assert_eq!(f32at(&tr, 0x0c + 8), 100.0, "endpos = start");
    assert_eq!((tr[0x36], tr[0x37]), (1, 1), "allsolid / startsolid");
    assert_eq!(
        i32::from_le_bytes(tr[0x30..0x34].try_into().unwrap()),
        Contents::WATER.bits()
    );
    assert_eq!(
        u64::from_le_bytes(tr[0x58..0x60].try_into().unwrap()),
        0x1234,
        "m_pEnt = the world entity"
    );

    // Starts inside but leaves: startsolid and fractionleftsolid, the rest is the engine's.
    let ray = line([64.0, 64.0, 64.0], [0.0, 0.0, 300.0]);
    let mut tr = miss_trace();
    let before = tr;
    assert!(apply(&vols, &ray, &mut tr, 0x1234));
    assert_eq!((tr[0x36], tr[0x37]), (0, 1));
    let mut expect = before;
    expect[0x37] = 1;
    expect[0x38..0x3c].copy_from_slice(&(64.0f32 / 300.0).to_le_bytes());
    assert_eq!(tr, expect);
}

#[test]
pub(crate) fn entity_array_global_is_found_and_agrees() {
    let Some(d) = server_dll() else {
        eprintln!("SERVER_DLL_X64 not set; skipping");
        return;
    };
    let pe = parse_pe(&d);
    let img = map_image(&d, &pe);
    unsafe {
        let g = entity_array_global(img.as_ptr()).expect("entity array global");
        let grva = g - img.as_ptr() as usize;
        let sec = pe
            .sections
            .iter()
            .find(|s| s.1 <= grva && grva < s.1 + s.2)
            .expect("outside sections");
        assert!(sec.0 == ".data" || sec.0 == ".bss", "in {}", sec.0);
    }
}

#[test]
pub(crate) fn world_entity_is_entry_zero_of_the_array() {
    let _g = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut array = [0usize; 8]; // entry 0: [?, pEntity, serial, ?]
    array[1] = 0xABCD;
    let global: usize = array.as_ptr() as usize;
    ENTITY_ARRAY_GLOBAL.store(std::ptr::addr_of!(global) as usize, Ordering::SeqCst);
    assert_eq!(unsafe { world_entity() }, 0xABCD);
    ENTITY_ARRAY_GLOBAL.store(0, Ordering::SeqCst);
    assert_eq!(unsafe { world_entity() }, 0);
}

#[test]
pub(crate) fn water_hit_names_the_world_entity_or_is_skipped() {
    let _g = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let vols = [water(1, [0.0; 3], [100.0; 3])];
    let mut ray = vec![0u8; 0x60];
    let put = |ray: &mut Vec<u8>, o: usize, v: f32| ray[o..o + 4].copy_from_slice(&v.to_le_bytes());
    for (o, v) in [
        (0x00, -50.0f32),
        (0x04, 50.0),
        (0x08, 50.0),
        (0x10, 200.0),
        (0x14, 0.0),
        (0x18, 0.0),
    ] {
        put(&mut ray, o, v);
    }
    ray[0x41] = 1;
    let mut tr = vec![0u8; 0x80];
    tr[0x2c..0x30].copy_from_slice(&1.0f32.to_le_bytes());
    // No entity known anywhere: leave the trace alone.
    assert!(!apply(&vols, &ray, &mut tr, 0));
    assert_eq!(tr[0x58..0x60], [0u8; 8]);
    // World known: the hit names it.
    assert!(apply(&vols, &ray, &mut tr, 0x1234));
    assert_eq!(
        usize::from_le_bytes(tr[0x58..0x60].try_into().unwrap()),
        0x1234
    );
}

pub(crate) fn make_ray(
    start: [f32; 3],
    delta: [f32; 3],
    off: [f32; 3],
    ext: [f32; 3],
    swept: bool,
) -> [u8; 0x50] {
    let mut r = [0u8; 0x50];
    for i in 0..3 {
        r[i * 4..i * 4 + 4].copy_from_slice(&start[i].to_le_bytes());
        r[0x10 + i * 4..0x14 + i * 4].copy_from_slice(&delta[i].to_le_bytes());
        r[0x20 + i * 4..0x24 + i * 4].copy_from_slice(&off[i].to_le_bytes());
        r[0x30 + i * 4..0x34 + i * 4].copy_from_slice(&ext[i].to_le_bytes());
    }
    r[0x40] = u8::from(ext == [0.0; 3]); // m_IsRay
    r[0x41] = u8::from(swept); // m_IsSwept
    r
}

/// A swept point ray from `start` along `delta`.
pub(crate) fn line(start: [f32; 3], delta: [f32; 3]) -> Vec<u8> {
    make_ray(start, delta, [0.0; 3], [0.0; 3], true).to_vec()
}

pub(crate) fn apply(vols: &[Volume], ray: &[u8], tr: &mut [u8], ent: usize) -> bool {
    unsafe { apply_water_to_trace(vols, ray.as_ptr(), tr.as_mut_ptr(), ent) }
}

pub(crate) fn apply_masked(
    vols: &[Volume],
    ray: &[u8],
    tr: &mut [u8],
    ent: usize,
    mask: Contents,
) -> bool {
    unsafe { apply_water_to_trace_masked(vols, ray.as_ptr(), tr.as_mut_ptr(), ent, mask) }
}

pub(crate) fn miss_trace() -> [u8; 0x70] {
    let mut t = [0u8; 0x70];
    t[0x2c..0x30].copy_from_slice(&1.0f32.to_le_bytes()); // fraction = 1
    t[0x58..0x60].copy_from_slice(&0xABCDu64.to_le_bytes()); // m_pEnt sentinel
    t
}

pub(crate) fn f32at(b: &[u8], o: usize) -> f32 {
    f32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

fn add3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

/// Slime for every `n`th id, water for the rest.
fn slime_every(id: impl TryInto<u32>, n: u32) -> Contents {
    if id.try_into().is_ok_and(|id| id % n == 0) {
        Contents::SLIME
    } else {
        Contents::WATER
    }
}

pub(crate) const fn cube() -> [Volume; 1] {
    [water(1, [0.0; 3], [128.0; 3])]
}

#[test]
pub(crate) fn trace_mask_selects_the_volumes_it_asks_for() {
    // A water volume and a slime volume side by side along the ray.
    let vols = [
        cube()[0],
        vol(2, [200.0, 0.0, 0.0], [328.0, 128.0, 128.0], Contents::SLIME),
    ];
    let ray = line([264.0, 64.0, 300.0], [0.0, 0.0, -300.0]);
    // Only the slime volume is under the ray: a water-only mask must leave it alone.
    let mut tr = miss_trace();
    assert!(!apply_masked(&vols, &ray, &mut tr, 0x1234, Contents::WATER));
    assert_eq!(tr, miss_trace());
    // A slime mask, and a mask with both, see it.
    for mask in [Contents::SLIME, Contents::WATER | Contents::SLIME] {
        let mut tr = miss_trace();
        assert!(apply_masked(&vols, &ray, &mut tr, 0x1234, mask));
        assert_eq!(
            i32::from_le_bytes(tr[0x30..0x34].try_into().unwrap()),
            Contents::SLIME.bits()
        );
    }
}

#[test]
pub(crate) fn volume_bounds_follow_every_change_and_never_reject_a_point_inside() {
    let a = cube()[0];
    let b = vol(
        2,
        [1000.0, -50.0, 20.0],
        [1100.0, 50.0, 60.0],
        Contents::SLIME,
    );
    let mut list = VolumeList::new();
    assert!(!list.may_contain([0.0; 3]), "empty list");
    list.push(a);
    list.push(b);
    assert_eq!(
        list.bounds,
        Some(([0.0, -50.0, 0.0], [1100.0, 128.0, 128.0]))
    );
    // Every point inside a volume must pass the box.
    let mut inside_count = 0;
    for x in (-200..1300).step_by(37) {
        for y in (-200..300).step_by(41) {
            for z in (-100..300).step_by(43) {
                let p = [x as f32, y as f32, z as f32];
                let inside = !contents_at(&list, p).is_empty();
                inside_count += usize::from(inside);
                assert!(!inside || list.may_contain(p), "{p:?}");
            }
        }
    }
    assert!(inside_count > 0);
    list.retain(|v| v.id != 2);
    assert_eq!(list.bounds, Some(([0.0; 3], [128.0; 3])));
    list.set(vec![b]);
    assert_eq!(list.bounds, Some((b.min, b.max)));
    list.clear();
    assert_eq!(list.bounds, None);
    assert!(!list.may_contain([0.0; 3]));
}

#[test]
pub(crate) fn rays_that_miss_the_bounds_never_touch_a_volume() {
    unsafe {
        let mut list = VolumeList::new();
        assert!(ray_misses_bounds(
            list.bounds,
            line([0.0; 3], [0.0, 0.0, -10.0]).as_ptr()
        ));
        list.push(cube()[0]);
        list.push(vol(
            2,
            [1000.0, -50.0, 20.0],
            [1100.0, 50.0, 60.0],
            Contents::SLIME,
        ));
        // A deterministic spread of rays: lines, hulls, long and short, starting in and out.
        let mut seed = 0x1234_5678_u32;
        let mut next = |range: f32| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            ((seed >> 8) as f32 / 16_777_216.0).mul_add(range, -(range / 2.0))
        };
        let (mut missed, mut hit) = (0, 0);
        for _ in 0..20_000 {
            let start = [next(2600.0) + 500.0, next(600.0), next(600.0) + 60.0];
            let delta = [next(1500.0), next(400.0), next(400.0)];
            let ext = if next(2.0) > 0.0 {
                [0.0; 3]
            } else {
                [next(60.0).abs(), next(60.0).abs(), next(60.0).abs()]
            };
            let ray = make_ray(start, delta, [0.0; 3], ext, true);
            let mut tr = miss_trace();
            let touched = apply_water_to_trace(&list, ray.as_ptr(), tr.as_mut_ptr(), 0x1234);
            if ray_misses_bounds(list.bounds, ray.as_ptr()) {
                missed += 1;
                assert!(!touched, "start {start:?} delta {delta:?} ext {ext:?}");
                assert_eq!(tr, miss_trace());
            } else {
                hit += 1;
            }
        }
        assert!(missed > 1000 && hit > 1000, "{missed} missed, {hit} not");
    }
}

#[test]
pub(crate) fn override_table_tracks_count_and_bounds() {
    let _g = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    override_clear();
    assert_eq!(OVERRIDE_COUNT.load(Ordering::Relaxed), 0);
    assert!(override_set(5, 1));
    assert!(override_set(9, 2));
    assert!(override_set(9, 3), "replacing keeps one entry");
    assert_eq!(OVERRIDE_COUNT.load(Ordering::Relaxed), 2);
    assert_eq!(override_get(9), Some(3));
    assert_eq!(override_get(8), None);
    assert_eq!(override_entries(), vec![(5, 1), (9, 3)]);
    assert!(override_remove(5));
    assert!(!override_remove(5), "already gone");
    assert_eq!(OVERRIDE_COUNT.load(Ordering::Relaxed), 1);
    // Out-of-range indices are refused and never found.
    for idx in [-1, MAX_ENTITIES as i32, i32::MAX, i32::MIN] {
        assert!(!override_set(idx, 1));
        assert_eq!(override_get(idx), None);
        assert!(!override_remove(idx));
    }
    assert_eq!(OVERRIDE_COUNT.load(Ordering::Relaxed), 1);
    override_clear();
    assert_eq!(OVERRIDE_COUNT.load(Ordering::Relaxed), 0);
    assert!(override_entries().is_empty());
}

#[test]
pub(crate) fn segment_box_entry_math() {
    let lo = [0.0; 3];
    let hi = [128.0; 3];
    // Falling onto the top face from above.
    let (t, axis, min_face) =
        segment_enters_box([64.0, 64.0, 300.0], [0.0, 0.0, -300.0], lo, hi).unwrap();
    assert!((t - 172.0 / 300.0).abs() < 1e-6);
    assert_eq!((axis, min_face), (2, false));
    // Rising into the bottom face from below.
    let (t, axis, min_face) =
        segment_enters_box([64.0, 64.0, -100.0], [0.0, 0.0, 300.0], lo, hi).unwrap();
    assert!((t - 1.0 / 3.0).abs() < 1e-6);
    assert_eq!((axis, min_face), (2, true));
    // Sideways through the max-X face.
    let (_, axis, min_face) =
        segment_enters_box([300.0, 64.0, 64.0], [-300.0, 0.0, 0.0], lo, hi).unwrap();
    assert_eq!((axis, min_face), (0, false));
    // Start inside, start on the boundary, too short, lateral miss, no motion.
    assert!(segment_enters_box([64.0; 3], [0.0, 0.0, -300.0], lo, hi).is_none());
    assert!(segment_enters_box([64.0, 64.0, 128.0], [0.0, 0.0, -300.0], lo, hi).is_none());
    assert!(segment_enters_box([64.0, 64.0, 300.0], [0.0, 0.0, -100.0], lo, hi).is_none());
    assert!(segment_enters_box([500.0, 64.0, 300.0], [0.0, 0.0, -300.0], lo, hi).is_none());
    assert!(segment_enters_box([64.0, 64.0, 300.0], [0.0; 3], lo, hi).is_none());
}

#[test]
pub(crate) fn newer_ray_layout_keeps_the_swept_flag_at_0x49() {
    unsafe {
        let vols = cube();
        let mut ray = make_ray(
            [64.0, 64.0, 300.0],
            [0.0, 0.0, -300.0],
            [0.0; 3],
            [0.0; 3],
            false,
        );
        ray[0x48] = 1; // m_IsRay
        ray[0x49] = 1; // m_IsSwept
        let mut tr = miss_trace();
        assert!(apply_water_to_trace_at(
            &vols,
            ray.as_ptr(),
            tr.as_mut_ptr(),
            0x1234,
            0x49,
            Contents::from_bits_retain(!0)
        ));
        // Reading the old position (0x41, zero here) would call it "not swept".
        let mut tr = miss_trace();
        assert!(!apply_water_to_trace_at(
            &vols,
            ray.as_ptr(),
            tr.as_mut_ptr(),
            0x1234,
            0x41,
            Contents::from_bits_retain(!0)
        ));
    }
}

#[test]
pub(crate) fn hull_pellet_line_reaches_the_floor_inside_the_volume() {
    // From a shotgun log: a 3 unit hull rested on the floor at x = 184.026, just outside
    // a volume whose +x face is at 183.906. The bullet's own line hits the floor inside.
    let probe = hull_surface_point(
        [184.026, -1343.636, -140.969],
        [0.0, 0.0, 1.0],
        [-51316.113, -2002.649, -24236.941],
        [3.0; 3],
    )
    .unwrap();
    assert!(probe[0] < 183.906, "x {}", probe[0]);
    assert!((probe[2] - (-140.969 - 3.0)).abs() < 1e-3, "z {}", probe[2]);
    // Moving away from the surface it stopped at: no probe.
    assert!(hull_surface_point([0.0; 3], [0.0, 0.0, 1.0], [0.0, 0.0, 10.0], [3.0; 3]).is_none());
}

#[test]
#[allow(clippy::too_many_lines)]
pub(crate) fn trace_is_rewritten_as_water_surface_hit() {
    unsafe {
        let vols = cube();

        // A line trace that would otherwise travel the full length.
        let ray = line([64.0, 64.0, 300.0], [0.0, 0.0, -300.0]);
        let mut tr = miss_trace();
        assert!(apply(&vols, &ray, &mut tr, 0x1234));
        let t = (172.0 + ENTRY_DEPTH) / 300.0;
        assert!((f32at(&tr, 0x2c) - t).abs() < 1e-6, "fraction");
        assert!(
            (f32at(&tr, 0x0c + 8) - (128.0 - ENTRY_DEPTH)).abs() < 1e-3,
            "endpos.z is just under the surface"
        );
        assert_eq!(f32at(&tr, 8), 300.0, "startpos.z");
        assert_eq!(
            (f32at(&tr, 0x18), f32at(&tr, 0x1c), f32at(&tr, 0x20)),
            (0.0, 0.0, 1.0),
            "normal"
        );
        assert_eq!(f32at(&tr, 0x24), 128.0, "plane.dist");
        assert_eq!((tr[0x28], tr[0x29]), (2, 0), "plane.type / signbits");
        assert_eq!(
            i32::from_le_bytes(tr[0x30..0x34].try_into().unwrap()),
            Contents::WATER.bits()
        );
        assert_eq!((tr[0x36], tr[0x37]), (0, 0), "allsolid / startsolid");
        assert_eq!(
            u64::from_le_bytes(tr[0x58..0x60].try_into().unwrap()),
            0x1234,
            "m_pEnt = the world entity"
        );
        let name = usize::from_le_bytes(tr[0x40..0x48].try_into().unwrap()) as *const c_char;
        assert_eq!(CStr::from_ptr(name).to_bytes(), b"water");

        // Entering from below flips the normal and sets signbits.
        let ray = line([64.0, 64.0, -100.0], [0.0, 0.0, 300.0]);
        let mut tr = miss_trace();
        assert!(apply(&vols, &ray, &mut tr, 0x1234));
        assert_eq!(f32at(&tr, 0x20), -1.0);
        assert_eq!(tr[0x29], 1 << 2);
        assert_eq!(f32at(&tr, 0x24), -0.0);

        // A hull trace is stopped by the box grown by its half-extents.
        let ray = make_ray(
            [64.0, 64.0, 300.0],
            [0.0, 0.0, -300.0],
            [0.0; 3],
            [8.0; 3],
            true,
        );
        let mut tr = miss_trace();
        assert!(apply(&vols, &ray, &mut tr, 0x1234));
        assert!((f32at(&tr, 0x2c) - 164.0 / 300.0).abs() < 1e-6);
        assert!(
            (f32at(&tr, 0x0c + 8) - 136.0).abs() < 1e-3,
            "hull origin rests 8 above"
        );
        assert_eq!(
            f32at(&tr, 0x24),
            128.0,
            "plane is the real face, not the grown one"
        );

        // The engine already hit something nearer: leave it alone.
        let ray = line([64.0, 64.0, 300.0], [0.0, 0.0, -300.0]);
        let mut tr = miss_trace();
        tr[0x2c..0x30].copy_from_slice(&0.3f32.to_le_bytes());
        let before = tr;
        assert!(!apply(&vols, &ray, &mut tr, 0x1234));
        assert_eq!(tr, before);

        // Not swept, or misses the box: unchanged.
        for ray in [
            make_ray(
                [64.0, 64.0, 300.0],
                [0.0, 0.0, -300.0],
                [0.0; 3],
                [0.0; 3],
                false,
            )
            .to_vec(),
            line([900.0, 64.0, 300.0], [0.0, 0.0, -300.0]),
        ] {
            let mut tr = miss_trace();
            let before = tr;
            assert!(!apply(&vols, &ray, &mut tr, 0x1234));
            assert_eq!(tr, before);
        }
    }
}

pub(crate) static TRACE_ARGS: Mutex<Vec<(u32, usize, usize, usize)>> = Mutex::new(Vec::new());

/// Stand-in for the engine's `TraceRay`: records its arguments, reports a miss.
pub(crate) unsafe extern "win64" fn fake_trace_ray(
    _this: *mut c_void,
    ray: *const u8,
    mask: u32,
    filter: *mut c_void,
    trace: *mut u8,
) {
    unsafe {
        TRACE_ARGS
            .lock()
            .unwrap()
            .push((mask, ray as usize, filter as usize, trace as usize));
        trace.add(0x2c).cast::<f32>().write_unaligned(1.0);
    }
}

#[test]
pub(crate) fn trace_hook_through_vtable() {
    let _g = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    unsafe {
        let mut vtable: Vec<usize> = vec![fake_trace_ray as *const () as usize; 8];
        let mut object: [usize; 2] = [vtable.as_mut_ptr() as usize, 0];
        let trace_obj = object.as_mut_ptr().cast::<c_void>();

        volumes().set(cube().to_vec());
        TRACE_ARGS.lock().unwrap().clear();
        let hits_before = TRACE_HITS.load(Ordering::Relaxed);

        let hook = hook_trace_ray as *const () as usize;
        let (entry, original) = install_slot_hook(
            trace_obj,
            TRACE_RAY_SLOT,
            hook,
            &ORIGINAL_TRACE_RAY,
            &TEST_PLATFORM,
        )
        .expect("install");
        assert_eq!(entry, vtable.as_ptr().add(TRACE_RAY_SLOT) as usize);
        assert_eq!(vtable[TRACE_RAY_SLOT], hook);
        assert_eq!(vtable[0], original, "other slots untouched");

        let ray = line([64.0, 64.0, 300.0], [0.0, 0.0, -300.0]);
        let call = |mask: u32, tr: &mut [u8]| {
            let f: unsafe extern "win64" fn(*mut c_void, *const u8, u32, *mut c_void, *mut u8) =
                std::mem::transmute(vtable[TRACE_RAY_SLOT]);
            f(
                trace_obj,
                ray.as_ptr(),
                mask,
                0x1234 as *mut c_void,
                tr.as_mut_ptr(),
            );
        };

        // Water in the mask: the miss becomes a water-surface hit.
        let mut tr = miss_trace();
        call(Contents::WATER.bits() as u32 | 0x10, &mut tr);
        assert!((f32at(&tr, 0x2c) - (172.0 + ENTRY_DEPTH) / 300.0).abs() < 1e-6);
        assert_eq!(
            i32::from_le_bytes(tr[0x30..0x34].try_into().unwrap()),
            Contents::WATER.bits()
        );

        // No water in the mask (e.g. a solid-only trace): untouched.
        let mut tr = miss_trace();
        call(0x1, &mut tr);
        assert_eq!(f32at(&tr, 0x2c), 1.0);

        // No volumes: pure pass-through even with water in the mask.
        volumes().clear();
        let mut tr = miss_trace();
        call(Contents::WATER.bits() as u32, &mut tr);
        assert_eq!(f32at(&tr, 0x2c), 1.0);

        // The original saw every argument, including the stack-passed 5th.
        let seen = TRACE_ARGS.lock().unwrap().clone();
        assert_eq!(seen.len(), 3);
        for (_, ray_ptr, filter, trace_ptr) in &seen {
            assert_eq!(*ray_ptr, ray.as_ptr() as usize);
            assert_eq!(*filter, 0x1234);
            assert_ne!(*trace_ptr, 0);
        }
        assert_eq!(seen[0].0, Contents::WATER.bits() as u32 | 0x10);
        assert_eq!(TRACE_HITS.load(Ordering::Relaxed), hits_before + 1);

        uninstall_slot_hook(entry, hook, original, &TEST_PLATFORM);
        assert_eq!(
            vtable[TRACE_RAY_SLOT], original,
            "uninstall restores the slot"
        );
    }
}

#[test]
pub(crate) fn lane_scan_agrees_with_the_plain_scan() {
    let mut state = 0x2545_F491_u32;
    let mut next = move || {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (state >> 8) as f32 / 16_777_216.0
    };
    let mut list = VolumeList::new();
    for id in 0..40 {
        let lo = [next() * 800.0, next() * 800.0, next() * 800.0];
        let size = [next() * 200.0, next() * 200.0, next() * 200.0];
        list.push(vol(id, lo, add3(lo, size), slime_every(id, 3)));
    }
    let mut hits = 0;
    for _ in 0..20_000 {
        let p = [next() * 1000.0, next() * 1000.0, next() * 1000.0];
        let expected = contents_at(&list, p);
        hits += usize::from(!expected.is_empty());
        assert_eq!(list.contents_at(p), expected, "{p:?}");
    }
    assert!(hits > 0);
    // Edges are inside, NaN is not.
    let v = list[0];
    assert!(list.contents_at(v.min).contains(v.contents));
    assert_eq!(list.contents_at([f32::NAN, 1.0, 1.0]), Contents::empty());
    list.retain(|v| v.id == 1);
    assert_eq!(list.contents_at(list[0].min), list[0].contents);
    list.clear();
    assert_eq!(list.contents_at([0.0; 3]), Contents::empty());
}

#[test]
pub(crate) fn every_point_query_agrees_with_the_plain_scan_at_every_size() {
    let mut state = 0x9E37_79B9_u32;
    let mut next = move || {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (state >> 8) as f32 / 16_777_216.0
    };
    for count in [0usize, 1, 5, 16, 17, 40, 256, 300] {
        let mut list = VolumeList::new();
        for id in 0..count {
            let lo = [next() * 4000.0, next() * 4000.0, next() * 4000.0];
            let size = [next() * 300.0, next() * 300.0, next() * 300.0];
            list.push(vol(id as u32, lo, add3(lo, size), slime_every(id, 4)));
        }
        let mut inside = 0;
        for round in 0..5_000 {
            // Half the points are inside some volume, half anywhere.
            let p = if round % 2 == 0 && count > 0 {
                let v = list[round % count];
                [
                    (v.max[0] - v.min[0]).mul_add(next(), v.min[0]),
                    (v.max[1] - v.min[1]).mul_add(next(), v.min[1]),
                    (v.max[2] - v.min[2]).mul_add(next(), v.min[2]),
                ]
            } else {
                [next() * 4300.0, next() * 4300.0, next() * 4300.0]
            };
            assert_eq!(list.contents_at(p), contents_at(&list, p), "{count} {p:?}");
            assert_eq!(
                list.contains(p),
                point_in_volumes(&list, p),
                "{count} {p:?}"
            );
            assert_eq!(list.surface_z(p), surface_z_at(&list, p), "{count} {p:?}");
            inside += usize::from(list.contains(p));
        }
        assert!(count == 0 || inside > 0, "{count}");
        // Corners are inside, NaN is not.
        for v in list.iter().take(8) {
            assert!(list.contains(v.min) && list.contains(v.max));
        }
        assert!(!list.contains([f32::NAN; 3]));
    }
}

#[test]
pub(crate) fn trails_that_miss_the_bounds_are_never_clipped() {
    let mut state = 0x1234_5678_u32;
    let mut next = move || {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (state >> 8) as f32 / 16_777_216.0
    };
    let mut list = VolumeList::new();
    for id in 0..6 {
        let lo = [next() * 500.0, next() * 500.0, next() * 500.0];
        list.push(water(id, lo, add3(lo, [90.0; 3])));
    }
    let mut misses = 0;
    for _ in 0..20_000 {
        let f = [
            next() * 900.0 - 150.0,
            next() * 900.0 - 150.0,
            next() * 900.0 - 150.0,
        ];
        let t = [
            next() * 900.0 - 150.0,
            next() * 900.0 - 150.0,
            next() * 900.0 - 150.0,
        ];
        let d = [t[0] - f[0], t[1] - f[1], t[2] - f[2]];
        if segment_misses_bounds(list.bounds, f, d, [0.0; 3]) {
            misses += 1;
            assert_eq!(
                clip_bubble_trail(&list, f, t),
                BubbleClip::Keep,
                "{f:?} {t:?}"
            );
        }
    }
    assert!(misses > 0);
    assert!(segment_misses_bounds(None, [0.0; 3], [1.0; 3], [0.0; 3]));
}

#[test]
pub(crate) fn hooks_read_the_snapshot_published_by_every_edit() {
    let _g = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    volumes().clear();
    assert!(snapshot().is_none());
    volumes().push(water(1, [0.0; 3], [10.0; 3]));
    assert_eq!(snapshot().map(|v| v.len()), Some(1));
    {
        // A read through the guard does not republish; an edit does, once released.
        let before = snapshot().unwrap();
        let guard = volumes();
        assert_eq!(guard.len(), 1);
        drop(guard);
        assert!(Arc::ptr_eq(&before, &snapshot().unwrap()));
        let mut edit = volumes();
        edit.retain(|v| v.id != 1);
        assert_eq!(snapshot().map(|v| v.len()), Some(1), "published on release");
        drop(edit);
        assert!(snapshot().is_none());
    }
}

#[test]
pub(crate) fn contents_hook_adds_water_inside_volumes() {
    let _g = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    unsafe {
        // A fake IEngineTrace: object -> vtable -> [GetPointContents, ...]
        let mut vtable: Vec<usize> = vec![fake_get_point_contents as *const () as usize; 8];
        let mut object: [usize; 2] = [vtable.as_mut_ptr() as usize, 0];
        let trace = object.as_mut_ptr().cast::<c_void>();

        volumes().clear();
        volumes().push(water(1, [0.0, 0.0, 0.0], [128.0, 128.0, 128.0]));
        let hits_before = VOLUME_HITS.load(Ordering::Relaxed);

        let (slot, original) = install_contents_hook(trace, &TEST_PLATFORM).expect("install");
        assert_eq!(slot, vtable.as_ptr() as usize);
        assert_eq!(original, fake_get_point_contents as *const () as usize);
        assert_ne!(vtable[0], original, "slot 0 must now point at the hook");
        assert_eq!(vtable[1], original, "other slots must be untouched");

        // Call it exactly the way the engine does: through the vtable.
        let call = |p: [f32; 3]| -> c_int {
            let f: unsafe extern "win64" fn(*mut c_void, *const f32, usize, usize) -> c_int =
                std::mem::transmute(vtable[0]);
            f(trace, p.as_ptr(), 7, 9)
        };
        assert_eq!(call([64.0, 64.0, 64.0]), 0x1 | Contents::WATER.bits()); // inside: water added
        assert_eq!(call([500.0, 0.0, 0.0]), 0x1); // outside: untouched
        assert_eq!(
            *GPC_ARGS.lock().unwrap(),
            (7, 9),
            "extra args must be forwarded"
        );
        assert_eq!(VOLUME_HITS.load(Ordering::Relaxed), hits_before + 1);

        volumes().clear();
        assert_eq!(call([64.0, 64.0, 64.0]), 0x1); // no volumes: pure pass-through

        uninstall_contents_hook(slot, original, &TEST_PLATFORM);
        assert_eq!(vtable[0], original, "uninstall must restore slot 0");
    }
}
