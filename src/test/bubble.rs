// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67

use super::*;
use std::sync::atomic::Ordering;

#[test]
pub(crate) fn bubble_trail_signature_is_unique_in_server_dll() {
    let Some(d) = server_dll() else {
        eprintln!("SERVER_DLL_X64 not set; skipping");
        return;
    };
    let pat = parse_sig(BUBBLE_TRAIL_SIG);
    let pe = parse_pe(&d);
    let (_, text_rva, _, text_raw, text_size, _) =
        pe.sections.iter().find(|s| s.0 == ".text").unwrap().clone();
    let hits = find_all(&d[text_raw..text_raw + text_size], &pat);
    assert_eq!(hits.len(), 1, "must match exactly once");
    println!("UTIL_BubbleTrail at server.dll+{:#x}", text_rva + hits[0]);
    // The stolen bytes end on an instruction boundary: the next byte is
    // the `movss xmm1,[rcx+8]` that follows the 7-byte `sub rsp,0xe0`.
    let o = text_raw + hits[0] + BUBBLE_TRAIL_STOLEN;
    assert_eq!(&d[o..o + 3], &[0xF3, 0x0F, 0x10]);
}

#[test]
pub(crate) fn bubble_trail_clip_math() {
    let v = [Volume {
        id: 1,
        min: [0.0; 3],
        max: [128.0; 3],
        contents: CONTENTS_WATER,
    }];
    // Fully inside, or nowhere near: unchanged.
    assert_eq!(
        clip_bubble_trail(&v, [10.0; 3], [100.0; 3]),
        BubbleClip::Keep
    );
    assert_eq!(
        clip_bubble_trail(&v, [500.0; 3], [600.0; 3]),
        BubbleClip::Keep
    );
    assert_eq!(
        clip_bubble_trail(&v, [5.0, 5.0, 5.0], [5.0, 5.0, 5.0]),
        BubbleClip::Keep
    );

    // Out through a side wall: 64 inside, 1000 units of flight.
    let BubbleClip::Clipped(a, b, frac) =
        clip_bubble_trail(&v, [64.0, 64.0, 64.0], [1064.0, 64.0, 64.0])
    else {
        panic!("expected a clip");
    };
    assert_eq!(a, [64.0, 64.0, 64.0]);
    assert!(
        (b[0] - 127.5).abs() < 1e-3,
        "{b:?}: exit point nudged 0.5 inside"
    );
    assert!((frac - 63.5 / 1000.0).abs() < 1e-5, "{frac}");

    // From outside through the box: clipped at both walls.
    let BubbleClip::Clipped(a, b, frac) =
        clip_bubble_trail(&v, [-100.0, 64.0, 64.0], [300.0, 64.0, 64.0])
    else {
        panic!("expected a clip");
    };
    assert!((a[0] - 0.5).abs() < 1e-3 && (b[0] - 127.5).abs() < 1e-3);
    assert!((frac - 127.0 / 400.0).abs() < 1e-5);

    // Grazing the wall leaves less than the nudge: nothing to draw.
    assert_eq!(
        clip_bubble_trail(&v, [127.9, 64.0, 64.0], [1000.0, 64.0, 64.0]),
        BubbleClip::Skip
    );

    // Two volumes: the one the trail starts in wins.
    let two = [
        Volume {
            id: 1,
            min: [200.0, 0.0, 0.0],
            max: [328.0, 128.0, 128.0],
            contents: CONTENTS_WATER,
        },
        Volume {
            id: 2,
            min: [0.0; 3],
            max: [128.0; 3],
            contents: CONTENTS_WATER,
        },
    ];
    let BubbleClip::Clipped(_, b, _) =
        clip_bubble_trail(&two, [64.0, 64.0, 64.0], [1000.0, 64.0, 64.0])
    else {
        panic!("expected a clip");
    };
    assert!((b[0] - 127.5).abs() < 1e-3);
}

pub(crate) type BubbleCall = ([f32; 3], [f32; 3], i32);

pub(crate) static BUBBLE_CALLS: Mutex<Vec<BubbleCall>> = Mutex::new(Vec::new());

pub(crate) unsafe extern "win64" fn fake_bubble_trail(
    from: *const f32,
    to: *const f32,
    count: c_int,
) {
    unsafe {
        BUBBLE_CALLS.lock().unwrap().push((
            [*from, *from.add(1), *from.add(2)],
            [*to, *to.add(1), *to.add(2)],
            count,
        ));
    }
}

#[test]
pub(crate) fn bubble_hook_clips_count_and_releases_the_lock() {
    let _g = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    unsafe {
        BUBBLE_TRAMPOLINE.store(fake_bubble_trail as *const () as usize, Ordering::SeqCst);
        BUBBLE_CALLS.lock().unwrap().clear();
        let clips = BUBBLE_CLIPS.load(Ordering::Relaxed);
        volumes().set(vec![Volume {
            id: 1,
            min: [0.0; 3],
            max: [128.0; 3],
            contents: CONTENTS_WATER,
        }]);

        let from = [64.0f32, 64.0, 64.0];
        let to = [1064.0f32, 64.0, 64.0];
        hook_bubble_trail(from.as_ptr(), to.as_ptr(), 125);
        {
            let calls = BUBBLE_CALLS.lock().unwrap();
            assert_eq!(calls.len(), 1);
            assert!((calls[0].1[0] - 127.5).abs() < 1e-3);
            assert_eq!(calls[0].2, 8, "ceil(125 * 0.0635)");
            drop(calls);
        }
        assert_eq!(BUBBLE_CLIPS.load(Ordering::Relaxed), clips + 1);

        // Inside the volume the original must be able to take the lock.
        // (The fake just proves the lock is free at call time.)
        unsafe extern "win64" fn lock_probe(_: *const f32, _: *const f32, _: c_int) {
            assert!(
                SNAPSHOT.try_write().is_ok(),
                "the volume snapshot is held across the original call"
            );
        }
        BUBBLE_TRAMPOLINE.store(lock_probe as *const () as usize, Ordering::SeqCst);
        hook_bubble_trail(from.as_ptr(), to.as_ptr(), 125);

        // Unrelated trail: passed through untouched.
        BUBBLE_TRAMPOLINE.store(fake_bubble_trail as *const () as usize, Ordering::SeqCst);
        BUBBLE_CALLS.lock().unwrap().clear();
        let a = [500.0f32, 0.0, 0.0];
        let b = [900.0f32, 0.0, 0.0];
        hook_bubble_trail(a.as_ptr(), b.as_ptr(), 50);
        assert_eq!(BUBBLE_CALLS.lock().unwrap()[0], (a, b, 50));

        // Skip: grazing trail makes no call at all.
        BUBBLE_CALLS.lock().unwrap().clear();
        let g = [127.9f32, 64.0, 64.0];
        hook_bubble_trail(g.as_ptr(), to.as_ptr(), 50);
        assert!(BUBBLE_CALLS.lock().unwrap().is_empty());

        volumes().clear();
    }
}

#[test]
pub(crate) fn install_patch_on_real_bubble_trail_bytes() {
    let Some(d) = server_dll() else {
        eprintln!("SERVER_DLL_X64 not set; skipping");
        return;
    };
    let pe = parse_pe(&d);
    let (_, _text_rva, _, text_raw, _, _) =
        pe.sections.iter().find(|s| s.0 == ".text").unwrap().clone();
    let pat = parse_sig(BUBBLE_TRAIL_SIG);
    let at = find_all(&d[text_raw..], &pat)[0];
    let func = &d[text_raw + at..][..0x8f];
    unsafe {
        let target = rwx(4096);
        std::ptr::copy_nonoverlapping(func.as_ptr(), target, func.len());
        let patch = install_patch(
            target,
            BUBBLE_TRAIL_SIG,
            BUBBLE_TRAIL_STOLEN,
            hook_bubble_trail as *const () as usize,
            &TEST_PLATFORM,
        )
        .expect("install_patch");
        let written = std::slice::from_raw_parts(target, BUBBLE_TRAIL_STOLEN);
        assert_eq!(&written[..6], &[0xFF, 0x25, 0, 0, 0, 0], "jmp [rip+0]");
        assert_eq!(
            u64::from_le_bytes(written[6..14].try_into().unwrap()),
            hook_bubble_trail as *const () as usize as u64
        );
        assert!(written[14..].iter().all(|&b| b == 0x90));
        // Trampoline = stolen bytes + absolute jump back past them.
        let t = std::slice::from_raw_parts(patch.trampoline as *const u8, BUBBLE_TRAIL_STOLEN + 14);
        assert_eq!(&t[..BUBBLE_TRAIL_STOLEN], &func[..BUBBLE_TRAIL_STOLEN]);
        assert_eq!(
            &t[BUBBLE_TRAIL_STOLEN..BUBBLE_TRAIL_STOLEN + 6],
            &[0xFF, 0x25, 0, 0, 0, 0]
        );
        assert_eq!(
            u64::from_le_bytes(t[BUBBLE_TRAIL_STOLEN + 6..].try_into().unwrap()),
            target as u64 + BUBBLE_TRAIL_STOLEN as u64
        );
        // A wrong prologue is refused and nothing is written.
        let bad = rwx(4096);
        std::ptr::write_bytes(bad, 0xCC, 64);
        assert!(install_patch(
            bad,
            BUBBLE_TRAIL_SIG,
            BUBBLE_TRAIL_STOLEN,
            1,
            &TEST_PLATFORM
        )
        .is_err());
        assert_eq!(*bad, 0xCC);

        uninstall_patch(&patch, &TEST_PLATFORM);
        assert_eq!(std::slice::from_raw_parts(target, func.len()), func);
    }
}
