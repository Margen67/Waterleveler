// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67

use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
pub(crate) fn client_trace_signature_and_ray_helper_match_once_in_client_dll() {
    let Some(d) = client_dll() else {
        eprintln!("CLIENT_DLL_X64 not set; skipping");
        return;
    };
    let pe = parse_pe(&d);
    let (_, _, _, text_raw, text_size, _) =
        pe.sections.iter().find(|s| s.0 == ".text").unwrap().clone();
    let text = &d[text_raw..text_raw + text_size];
    for sig in [CLIENT_TRACE_SIG, RAY_HELPER_SIG] {
        assert_eq!(find_all(text, &parse_sig(sig)).len(), 1, "{sig}");
    }
}

#[test]
pub(crate) fn signature_matcher_handles_wildcards() {
    let pat = parse_sig("01 ?? 03");
    assert_eq!(find_all(&[9, 1, 7, 3, 1, 8, 3, 1], &pat), vec![1, 4]);
    assert!(find_all(&[1, 2], &pat).is_empty());
}

/// RVA of the one place SIG matches in the file's executable sections.
pub(crate) fn sig_rva(d: &[u8], pe: &Pe) -> usize {
    let pat = parse_sig(SIG);
    let mut total = Vec::new();
    for (_, rva, _, raw, rawsz, chars) in &pe.sections {
        if chars & 0x2000_0000 != 0 {
            for off in find_all(&d[*raw..*raw + *rawsz], &pat) {
                total.push(rva + off);
            }
        }
    }
    assert_eq!(
        total.len(),
        1,
        "SetWaterLevel signature must match exactly once: {total:x?}"
    );
    total[0]
}

#[test]
pub(crate) fn signature_is_unique_in_server_dll() {
    let Some(d) = server_dll() else {
        eprintln!("SERVER_DLL_X64 not set; skipping");
        return;
    };
    let pe = parse_pe(&d);
    let pat = parse_sig(SIG);
    let mut total = Vec::new();
    for (name, rva, _, raw, rawsz, chars) in &pe.sections {
        if chars & 0x2000_0000 != 0 {
            for off in find_all(&d[*raw..*raw + *rawsz], &pat) {
                total.push((name.clone(), rva + off));
            }
        }
    }
    assert_eq!(total.len(), 1, "{total:?}");
    assert_eq!(total[0].0, ".text");
}

#[test]
pub(crate) fn scan_image_finds_the_function() {
    let Some(d) = server_dll() else { return };
    let pe = parse_pe(&d);
    let img = map_image(&d, &pe);
    let found = unsafe { scan_image(img.as_ptr()) }.expect("scan failed");
    assert_eq!(found as usize - img.as_ptr() as usize, sig_rva(&d, &pe));
}

#[test]
pub(crate) fn ray_helper_signature_gives_the_swept_offset() {
    let Some(d) = server_dll() else {
        eprintln!("SERVER_DLL_X64 not set; skipping");
        return;
    };
    let pe = parse_pe(&d);
    let img = map_image(&d, &pe);
    unsafe {
        let f = scan_sig(img.as_ptr(), RAY_HELPER_SIG).expect("ray helper must match exactly once");
        let off = ray_swept_offset_from(f);
        assert!(
            off == 0x41 || off == 0x49,
            "unexpected m_IsSwept offset {off:#x}"
        );
    }
}

#[test]
pub(crate) fn every_hook_target_can_be_detoured_and_restored() {
    let Some(d) = server_dll() else {
        eprintln!("SERVER_DLL_X64 not set; skipping");
        return;
    };
    let pe = parse_pe(&d);
    unsafe {
        let base = map_image_exec(&d, &pe);
        let slot = AtomicUsize::new(0);
        let mut targets = vec![
            (
                "SetWaterLevel",
                scan_sig(base, SIG),
                hook_set_water_level as *const (),
            ),
            (
                "UTIL_BubbleTrail",
                scan_sig(base, BUBBLE_TRAIL_SIG),
                hook_bubble_trail as *const (),
            ),
            (
                "FluidEndTouch",
                scan_sig(base, FLUID_END_TOUCH_SIG),
                hook_fluid_end_touch as *const (),
            ),
            (
                "CBaseTempEntity::Create",
                scan_sig(base, TE_CREATE_SIG),
                hook_te_create as *const (),
            ),
        ];
        for (i, hook) in EXPLOSION_HOOKS.iter().enumerate() {
            targets.push((
                "explosion check",
                find_explosion_target(base, i),
                *hook as *const (),
            ));
        }
        for (name, target, hook) in targets {
            let target = target.unwrap_or_else(|e| panic!("{name}: {e}"));
            let before = std::slice::from_raw_parts(target, 32).to_vec();
            let detour =
                install_detour(target, hook, &slot).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_ne!(slot.load(Ordering::SeqCst), 0, "{name}: trampoline");
            assert_ne!(
                std::slice::from_raw_parts(target, 5),
                &before[..5],
                "{name}"
            );
            drop(detour);
            assert_eq!(
                std::slice::from_raw_parts(target, 32),
                &before[..],
                "{name}"
            );
            slot.store(0, Ordering::SeqCst);
        }
    }
}

/// The straightforward matcher `find_all` replaced, as the reference.
fn find_all_naive(hay: &[u8], pat: &[Option<u8>]) -> Vec<usize> {
    if pat.is_empty() || hay.len() < pat.len() {
        return Vec::new();
    }
    (0..=hay.len() - pat.len())
        .filter(|&i| {
            pat.iter()
                .enumerate()
                .all(|(j, p)| p.is_none_or(|b| hay[i + j] == b))
        })
        .collect()
}

#[test]
pub(crate) fn find_all_agrees_with_the_naive_matcher() {
    // A tiny alphabet makes overlapping matches, matches at both ends and matches next to
    // wildcards common.
    let mut seed = 0x1234_5678_9ABC_DEF0u64;
    let mut next = move || {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        (seed >> 33) as usize
    };
    for round in 0..4000 {
        let hay: Vec<u8> = (0..next() % 200).map(|_| (next() % 3) as u8).collect();
        let pat: Vec<Option<u8>> = (0..1 + next() % 8)
            .map(|_| (next() % 4 != 0).then(|| (next() % 3) as u8))
            .collect();
        assert_eq!(
            find_all(&hay, &pat),
            find_all_naive(&hay, &pat),
            "round {round}: {hay:?} / {pat:?}"
        );
    }
}

#[test]
pub(crate) fn find_all_agrees_with_the_naive_matcher_on_the_real_server_dll() {
    let Some(d) = server_dll() else {
        eprintln!("SERVER_DLL_X64 not set; skipping");
        return;
    };
    let pe = parse_pe(&d);
    let (_, _, _, text_raw, text_size, _) =
        pe.sections.iter().find(|s| s.0 == ".text").unwrap().clone();
    let text = &d[text_raw..text_raw + text_size];
    for sig in [
        SIG,
        BUBBLE_TRAIL_SIG,
        FLUID_END_TOUCH_SIG,
        TE_CREATE_SIG,
        ENTITY_ARRAY_SIG,
        "48 8B ?? ?? ?? FF 15",
        "?? ?? ?? ??",
        "00 00 00 00 00 00 00 00 00 00 00 00",
    ] {
        let pat = parse_sig(sig);
        assert_eq!(find_all(text, &pat), find_all_naive(text, &pat), "{sig}");
    }
}
