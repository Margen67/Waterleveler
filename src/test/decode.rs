// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67

use super::*;

/// A signature's bytes with every wildcard set to zero, ready to patch.
fn bytes_of(sig: &str) -> Vec<u8> {
    parse_sig(sig).iter().map(|b| b.unwrap_or(0)).collect()
}

fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
    bytes[at..at + value.len()].copy_from_slice(value);
}

#[test]
pub(crate) fn a_global_is_the_rip_relative_load_in_the_match() {
    // mov rcx,[rip+0x100]
    let one = decode_bytes(&[0x48, 0x8B, 0x0D, 0x00, 0x01, 0x00, 0x00, 0x90], 0x1000);
    assert_eq!(only_global(&one), Ok(0x1000 + 7 + 0x100));
    assert_eq!(last_global(&one), Ok(0x1000 + 7 + 0x100));
    // ... then lea rdx,[rip+0x10]: two loads, the last one wins
    let two = decode_bytes(
        &[
            0x48, 0x8B, 0x0D, 0x00, 0x01, 0x00, 0x00, // mov rcx,[rip+0x100]
            0x48, 0x8D, 0x15, 0x10, 0x00, 0x00, 0x00, // lea rdx,[rip+0x10]
        ],
        0x1000,
    );
    assert_eq!(last_global(&two), Ok(0x1000 + 14 + 0x10));
    assert!(only_global(&two).is_err());
    // no load at all, or only one that is not of a global (a comparison)
    let none = decode_bytes(&[0x90, 0x48, 0x89, 0xD8], 0x1000);
    assert!(only_global(&none).is_err() && last_global(&none).is_err());
    // comiss xmm0,[rip+0x10] is a read of a constant, not a global load
    let constant = decode_bytes(&[0x0F, 0x2F, 0x05, 0x10, 0x00, 0x00, 0x00], 0x1000);
    assert!(last_global(&constant).is_err());
}

#[test]
pub(crate) fn decoding_stops_at_a_cut_instruction() {
    // a complete nop, then the first two bytes of a mov rcx,[rip+disp32]
    let code = decode_bytes(&[0x90, 0x48, 0x8B], 0x1000);
    assert_eq!(code.len(), 1);
}

#[test]
pub(crate) fn water_level_offset_needs_all_four_places_to_agree() {
    let mut code = bytes_of(SIG);
    for at in [25, 34, 46, 59] {
        put(&mut code, at, &[0xB3, 0x02]);
    }
    put(&mut code, 52, &[0xA8, 0x05]); // the vtable displacement of the NetworkStateChanged call
    assert_eq!(water_level_offset(&decode_bytes(&code, 0x1000)), Ok(0x2b3));

    let mut disagreeing = code.clone();
    put(&mut disagreeing, 46, &[0xAB, 0x02]);
    assert!(water_level_offset(&decode_bytes(&disagreeing, 0x1000)).is_err());

    // an offset outside what a game object could have
    let mut tiny = code.clone();
    for at in [25, 34, 46, 59] {
        put(&mut tiny, at, &[0x10, 0x00]);
    }
    assert!(water_level_offset(&decode_bytes(&tiny, 0x1000)).is_err());
    assert!(water_level_offset(&[]).is_err());
}

#[test]
pub(crate) fn fluid_layout_reads_the_flags_offset_and_the_event_slot() {
    let mut code = bytes_of(FLUID_END_TOUCH_SIG);
    put(&mut code, 106, &[0x98, 0x01]); // and [rsi+0x198], ~EFL_TOUCHING_FLUID
    put(&mut code, 139, &[0xE8, 0x02]); // call [rbx+0x2e8]
    assert_eq!(
        fluid_layout(&decode_bytes(&code, 0x1000)),
        Ok((0x198, 0x2e8 / 8))
    );
    // a call that is not at a slot boundary is not a vtable call
    let mut odd = code.clone();
    put(&mut odd, 139, &[0xE9, 0x02]);
    assert!(fluid_layout(&decode_bytes(&odd, 0x1000)).is_err());
    assert!(fluid_layout(&[]).is_err());
}

#[test]
pub(crate) fn linux_fluid_layout_reads_the_flag_and_the_loaded_event_slot() {
    // and dword [r13+0x1c0],0xfff7ffff; mov r12,[rax+0x2f0]; call [rax+0x38]; call r12
    let code: &[u8] = &[
        0x41, 0x81, 0xA5, 0xC0, 0x01, 0x00, 0x00, 0xFF, 0xFF, 0xF7, 0xFF, 0x4C, 0x8B, 0xA0, 0xF0,
        0x02, 0x00, 0x00, 0xFF, 0x50, 0x38, 0x41, 0xFF, 0xD4,
    ];
    assert_eq!(
        fluid_layout_via_load(&decode_bytes(code, 0x1000)),
        Ok((0x1c0, 0x2f0 / 8))
    );
    // no flag clear, no layout
    assert!(fluid_layout_via_load(&decode_bytes(&code[11..], 0x1000)).is_err());
    assert!(fluid_layout_via_load(&[]).is_err());
}

#[test]
pub(crate) fn linux_inline_ray_setup_gives_the_swept_offset() {
    let mut code = bytes_of(linux_sigs::RAY_INIT_SIG);
    // m_IsRay at rbp-0xb0, m_IsSwept at rbp-0xaf, the Ray_t at rbp-0xf0: +0x41
    put(&mut code, 2, &0xFFFF_FF50u32.to_le_bytes());
    let swept = code.windows(2).position(|w| w == [0x88, 0x85]).unwrap();
    put(&mut code, swept + 2, &0xFFFF_FF51u32.to_le_bytes());
    let lea = code.len() - 10;
    put(&mut code, lea + 3, &0xFFFF_FF10u32.to_le_bytes());
    assert_eq!(
        ray_swept_offset_inline(&decode_bytes(&code, 0x1000)),
        Ok(0x41)
    );
    // the x86-64 branch's Ray_t has the flags 8 bytes later
    put(&mut code, lea + 3, &0xFFFF_FF08u32.to_le_bytes());
    assert_eq!(
        ray_swept_offset_inline(&decode_bytes(&code, 0x1000)),
        Ok(0x49)
    );
    // m_IsSwept must be the byte after m_IsRay
    put(&mut code, swept + 2, &0xFFFF_FF60u32.to_le_bytes());
    assert!(ray_swept_offset_inline(&decode_bytes(&code, 0x1000)).is_err());
    assert!(ray_swept_offset_inline(&[]).is_err());
}

#[test]
pub(crate) fn ray_swept_offset_follows_the_frame_of_the_helper() {
    let mut code = bytes_of(RAY_HELPER_SIG);
    put(&mut code, 53, &[0xE9]); // mov byte [rax-0x17],1
                                 // one push (8) + sub rsp,0x90 - the Ray_t at rsp+0x40 gives -0x17 + 0x98 - 0x40
    assert_eq!(ray_swept_offset(&decode_bytes(&code, 0x1000)), Ok(0x41));
    put(&mut code, 53, &[0xF1]); // -0xf: the x86-64 branch's layout, 8 bytes further
    assert_eq!(ray_swept_offset(&decode_bytes(&code, 0x1000)), Ok(0x49));
    assert!(ray_swept_offset(&[]).is_err());
}

// ---- The decoder against the real binaries -------------------------------------------
//
// Before the decoder, each of these was read at a byte position counted by hand. Those
// positions are kept here, only as the reference to compare with.

unsafe fn legacy_rip(at: *const u8, disp_at: usize) -> usize {
    unsafe {
        let disp = at.add(disp_at).cast::<i32>().read_unaligned() as isize;
        at.offset(disp_at as isize + 4 + disp) as usize
    }
}

const unsafe fn legacy_u32(at: *const u8, i: usize) -> usize {
    unsafe { at.add(i).cast::<u32>().read_unaligned() as usize }
}

#[test]
pub(crate) fn decoded_windows_server_lookups_match_the_counted_offsets() {
    let Some(d) = server_dll() else {
        eprintln!("SERVER_DLL_X64 not set; skipping");
        return;
    };
    let pe = parse_pe(&d);
    let img = map_image(&d, &pe);
    unsafe {
        let base = img.as_ptr();
        let upd = scan_sig(base, UPDATE_WATER_STATE_SIG).unwrap();
        assert_eq!(
            last_global(&decode_match(upd, UPDATE_WATER_STATE_SIG)),
            Ok(legacy_rip(upd, 91))
        );
        let env = scan_sig(base, PHYSENV_SIG).unwrap();
        assert_eq!(
            only_global(&decode_match(env, PHYSENV_SIG)),
            Ok(legacy_rip(env, 3))
        );
        let all = scan_sig_all(base, ENTITY_ARRAY_SIG).unwrap();
        assert!(all.len() > 1);
        for at in all {
            assert_eq!(
                only_global(&decode_match(at, ENTITY_ARRAY_SIG)),
                Ok(legacy_rip(at, 8))
            );
        }
        let set = scan_sig(base, SIG).unwrap();
        let wl = legacy_u32(set, 25);
        assert!([25, 34, 46, 59].iter().all(|&i| legacy_u32(set, i) == wl));
        assert_eq!(water_level_offset(&decode_match(set, SIG)), Ok(wl));
        let end_touch = scan_sig(base, FLUID_END_TOUCH_SIG).unwrap();
        assert_eq!(
            fluid_layout(&decode_match(end_touch, FLUID_END_TOUCH_SIG)),
            Ok((legacy_u32(end_touch, 106), legacy_u32(end_touch, 139) / 8))
        );
        let ray = scan_sig(base, RAY_HELPER_SIG).unwrap();
        let disp = ray.add(53).cast::<i8>().read() as isize;
        assert_eq!(
            ray_swept_offset_from(ray),
            Ok((disp + 0x98 - 0x40) as usize)
        );
    }
}

#[test]
pub(crate) fn decoded_windows_client_lookups_match_the_counted_offsets() {
    let Some(d) = client_dll() else {
        eprintln!("CLIENT_DLL_X64 not set; skipping");
        return;
    };
    let pe = parse_pe(&d);
    let img = map_image(&d, &pe);
    unsafe {
        let at = scan_sig(img.as_ptr(), CLIENT_TRACE_SIG).unwrap();
        assert_eq!(
            trace_global_from_load(at).map(|g| g as usize),
            Ok(legacy_rip(at, 3))
        );
        let ray = scan_sig(img.as_ptr(), RAY_HELPER_SIG).unwrap();
        let disp = ray.add(53).cast::<i8>().read() as isize;
        assert_eq!(
            ray_swept_offset_from(ray),
            Ok((disp + 0x98 - 0x40) as usize)
        );
    }
}

#[test]
pub(crate) fn decoded_linux_lookups_match_the_counted_offsets() {
    let (Some(server), Some(client)) = (server_so(), client_so()) else {
        eprintln!("SERVER_SO_X64 / CLIENT_SO_X64 not set; skipping");
        return;
    };
    unsafe {
        let base = map_elf(&server);
        let upd = scan_sig(base, linux_sigs::UPDATE_WATER_STATE_SIG).unwrap();
        assert_eq!(
            last_global(&decode_match(upd, linux_sigs::UPDATE_WATER_STATE_SIG)),
            Ok(legacy_rip(upd, 95))
        );
        let env = scan_sig(base, linux_sigs::PHYSENV_SIG).unwrap();
        assert_eq!(
            only_global(&decode_match(env, linux_sigs::PHYSENV_SIG)),
            Ok(legacy_rip(env, 3))
        );
        let all = scan_sig_all(base, linux_sigs::ENTITY_ARRAY_SIG).unwrap();
        assert!(all.len() > 1);
        for at in all {
            assert_eq!(
                only_global(&decode_match(at, linux_sigs::ENTITY_ARRAY_SIG)),
                Ok(legacy_rip(at, 3))
            );
        }
        // the entity layout of the build, read from the matches
        let set = scan_sig(base, linux_sigs::SIG).unwrap();
        assert_eq!(
            water_level_offset_in(&decode_match(set, linux_sigs::SIG), 3),
            Ok(0x2db)
        );
        let end_touch = scan_sig(base, linux_sigs::FLUID_END_TOUCH_SIG).unwrap();
        assert_eq!(
            fluid_layout_via_load(&decode_match(end_touch, linux_sigs::FLUID_END_TOUCH_SIG)),
            Ok((0x1c0, 0x2f0 / 8))
        );
        assert_eq!(server_ray_swept_offset(base), Ok(0x41));
        let base = map_elf(&client);
        assert_eq!(client_ray_swept_offset(base), Ok(0x41));
        let at = scan_sig(base, linux_sigs::CLIENT_TRACE_SIG).unwrap();
        assert_eq!(
            last_global(&decode_match(at, linux_sigs::CLIENT_TRACE_SIG)),
            Ok(legacy_rip(at, 3))
        );
    }
}
