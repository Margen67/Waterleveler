// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67

use super::*;

pub(crate) const FUNC_LEN: usize = 0xbb;

pub(crate) fn server_so() -> Option<Vec<u8>> {
    let path = std::env::var("SERVER_SO_X64").ok()?;
    std::fs::read(path).ok()
}

/// Virtual addresses of every match of `sig` in the executable segments of a 64-bit ELF.
pub(crate) fn elf_matches(d: &[u8], sig: &str) -> Vec<usize> {
    let rd = |o: usize, n: usize| {
        usize::try_from(u64::from_le_bytes({
            let mut b = [0u8; 8];
            b[..n].copy_from_slice(&d[o..o + n]);
            b
        }))
        .unwrap()
    };
    assert_eq!(&d[..4], b"\x7fELF");
    let (phoff, phentsize, phnum) = (rd(0x20, 8), rd(0x36, 2), rd(0x38, 2));
    let pat = parse_sig(sig);
    let mut out = Vec::new();
    for i in 0..phnum {
        let ph = phoff + i * phentsize;
        if rd(ph, 4) == 1 && rd(ph + 4, 4) & 1 != 0 {
            let (off, va, filesz) = (rd(ph + 8, 8), rd(ph + 0x10, 8), rd(ph + 0x20, 8));
            out.extend(
                find_all(&d[off..off + filesz], &pat)
                    .into_iter()
                    .map(|m| va + m),
            );
        }
    }
    out
}

#[test]
pub(crate) fn linux_signatures_are_unique_in_server_so() {
    let Some(d) = server_so() else {
        eprintln!("SERVER_SO_X64 not set; skipping");
        return;
    };
    use linux_sigs as l;
    let mut sigs = vec![
        ("SetWaterLevel", l::SIG),
        ("UpdateWaterState", l::UPDATE_WATER_STATE_SIG),
        ("FluidEndTouch", l::FLUID_END_TOUCH_SIG),
        ("UTIL_BubbleTrail", l::BUBBLE_TRAIL_SIG),
        ("DispatchEffect", l::DISPATCH_EFFECT_SIG),
        ("CBaseTempEntity::Create", l::TE_CREATE_SIG),
        ("physenv user", l::PHYSENV_SIG),
    ];
    for t in l::EXPLOSION_TARGETS {
        sigs.push(("explosion target", t[0]));
    }
    for (name, sig) in sigs {
        let m = elf_matches(&d, sig);
        assert_eq!(m.len(), 1, "{name}: {} matches", m.len());
    }
    // the entity array idiom has many sites, all naming one global
    let sites = elf_matches(&d, l::ENTITY_ARRAY_SIG);
    assert!(sites.len() > 1);
}

pub(crate) fn client_so() -> Option<Vec<u8>> {
    let path = std::env::var("CLIENT_SO_X64").ok()?;
    std::fs::read(path).ok()
}

#[test]
pub(crate) fn linux_client_trace_signature_finds_the_enginetrace_variable() {
    let Some(d) = client_so() else {
        eprintln!("CLIENT_SO_X64 not set; skipping");
        return;
    };
    let m = elf_matches(&d, linux_sigs::CLIENT_TRACE_SIG);
    assert_eq!(m.len(), 1, "{} matches", m.len());
    unsafe {
        let base = map_elf(&d);
        let at = scan_sig(base, linux_sigs::CLIENT_TRACE_SIG).expect("matches once");
        assert_eq!(at as usize - base as usize, m[0]);
        // `C_BaseEntity::UpdateWaterState` loads the `enginetrace` variable at 0x12270b0
        // in this dev-branch client.so (the same one its `TraceRay` calls use).
        let var = trace_global_from_load(at);
        assert_eq!(var as usize - base as usize, 0x0122_70b0);
    }
}

/// Maps the `PT_LOAD` segments of a 64-bit ELF into one anonymous region at their virtual
/// addresses (no relocations, nothing is executed) and returns its base.
pub(crate) unsafe fn map_elf(d: &[u8]) -> *mut u8 {
    unsafe {
        let rd = |o: usize, n: usize| {
            let mut b = [0u8; 8];
            b[..n].copy_from_slice(&d[o..o + n]);
            usize::try_from(u64::from_le_bytes(b)).unwrap()
        };
        let (phoff, phentsize, phnum) = (rd(0x20, 8), rd(0x36, 2), rd(0x38, 2));
        let mut end = 0;
        for i in 0..phnum {
            let ph = phoff + i * phentsize;
            if rd(ph, 4) == 1 {
                end = end.max(rd(ph + 0x10, 8) + rd(ph + 0x28, 8));
            }
        }
        let size = end.div_ceil(4096) * 4096;
        let base = mmap(std::ptr::null_mut(), size, 7, 0x22, -1, 0).cast::<u8>();
        assert!(base as isize != -1);
        for i in 0..phnum {
            let ph = phoff + i * phentsize;
            if rd(ph, 4) == 1 {
                let (off, va, filesz) = (rd(ph + 8, 8), rd(ph + 0x10, 8), rd(ph + 0x20, 8));
                std::ptr::copy_nonoverlapping(d[off..].as_ptr(), base.add(va), filesz);
            }
        }
        base
    }
}

#[test]
pub(crate) fn linux_patches_install_on_the_mapped_server_so() {
    let Some(d) = server_so() else {
        eprintln!("SERVER_SO_X64 not set; skipping");
        return;
    };
    use linux_sigs as l;
    unsafe {
        let base = map_elf(&d);
        // the executable segments back to r-x so the real make_writable path is used
        extern "C" {
            fn mprotect(addr: *mut c_void, len: usize, prot: i32) -> i32;
        }
        let rva_of = |at: *mut u8| at as usize - base as usize;
        let text = scan_sig(base, l::SIG).expect("SetWaterLevel");
        assert_eq!(rva_of(text), 0x00bd_9cf0);
        let page = (text as usize & !4095) as *mut c_void;
        assert_eq!(mprotect(page, 4096, 5), 0, "r-x");

        let before = std::slice::from_raw_parts(text, 32).to_vec();
        let slot = std::sync::atomic::AtomicUsize::new(0);
        let detour = install_detour(text, hook_set_water_level as *const (), &slot)
            .expect("detour SetWaterLevel");
        assert_ne!(
            slot.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "trampoline recorded"
        );
        assert_ne!(std::slice::from_raw_parts(text, 5), &before[..5]);
        // the page is r-x again
        let perms = std::fs::read_to_string("/proc/self/maps").unwrap();
        let line = perms
            .lines()
            .find(|l| {
                let (lo, hi) = l
                    .split_whitespace()
                    .next()
                    .unwrap()
                    .split_once('-')
                    .unwrap();
                (usize::from_str_radix(lo, 16).unwrap()..usize::from_str_radix(hi, 16).unwrap())
                    .contains(&(text as usize))
            })
            .unwrap();
        assert_eq!(line.split_whitespace().nth(1), Some("r-xp"));
        drop(detour);
        assert_eq!(std::slice::from_raw_parts(text, 32), &before[..]);

        // CBaseTempEntity::Create: its prologue holds a rip-relative lea, which the
        // trampoline has to rewrite
        let create = scan_sig(base, l::TE_CREATE_SIG).expect("Create");
        let before = std::slice::from_raw_parts(create, 32).to_vec();
        let detour =
            install_detour(create, hook_te_create as *const (), &slot).expect("detour Create");
        assert_ne!(std::slice::from_raw_parts(create, 5), &before[..5]);
        drop(detour);
        assert_eq!(std::slice::from_raw_parts(create, 32), &before[..]);

        // globals (the tests build with the Windows constants, so the Linux displacement
        // positions are spelled out here: 95 / 3 / 3)
        let upd = scan_sig(base, l::UPDATE_WATER_STATE_SIG).expect("UpdateWaterState");
        let disp = upd.add(95).cast::<i32>().read_unaligned() as isize;
        assert_eq!(rva_of(upd.offset(95 + 4 + disp)), 0x01a7_3c58);
        let env = scan_sig(base, l::PHYSENV_SIG).expect("physenv user");
        assert_eq!(rva_of(physenv_global_from(env) as *mut u8), 0x0196_a670);
        for at in scan_sig_all(base, l::ENTITY_ARRAY_SIG).expect("entity array idiom") {
            let disp = at.add(3).cast::<i32>().read_unaligned() as isize;
            assert_eq!(rva_of(at.offset(7 + disp)), 0x0189_8d50);
        }
    }
}
