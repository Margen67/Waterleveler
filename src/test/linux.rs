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
pub(crate) fn relocated_rip_lea_loads_the_same_address() {
    // lea r14,[rip+0x1000] at offset 8 of a function at 0x5000_0000
    let mut code = vec![0x90u8; 8];
    code.extend_from_slice(&[0x4C, 0x8D, 0x35, 0x00, 0x10, 0x00, 0x00]);
    let abs = relocate_rip_lea(&code, 8, 0x5000_0000).expect("a lea");
    assert_eq!(&abs[..2], &[0x49, 0xBE], "movabs r14");
    assert_eq!(
        u64::from_le_bytes(abs[2..].try_into().unwrap()),
        0x5000_0000 + 8 + 7 + 0x1000
    );
    // lea rax,[rip-0x10]: no REX.B/R, register 0, negative displacement
    let code = [0x48, 0x8D, 0x05, 0xF0, 0xFF, 0xFF, 0xFF];
    let abs = relocate_rip_lea(&code, 0, 0x1000).unwrap();
    assert_eq!(&abs[..2], &[0x48, 0xB8]);
    assert_eq!(
        u64::from_le_bytes(abs[2..].try_into().unwrap()),
        0x1000 + 7 - 0x10
    );
    // not a rip-relative lea
    assert!(relocate_rip_lea(&[0x48, 0x89, 0xE5, 0, 0, 0, 0], 0, 0).is_none());
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
        sigs.push(("explosion target", t[0].0));
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

        let before = std::slice::from_raw_parts(text, 14).to_vec();
        let patch = install_patch(
            text,
            l::SIG,
            14,
            hook_set_water_level as *const () as usize,
            &PLATFORM,
        )
        .expect("install SetWaterLevel");
        let written = std::slice::from_raw_parts(text, 14);
        assert_eq!(&written[..6], &[0xFF, 0x25, 0, 0, 0, 0]);
        assert_eq!(
            u64::from_le_bytes(written[6..14].try_into().unwrap()),
            hook_set_water_level as *const () as usize as u64
        );
        let tramp = std::slice::from_raw_parts(patch.trampoline as *const u8, 28);
        assert_eq!(&tramp[..14], &before[..]);
        assert_eq!(
            u64::from_le_bytes(tramp[20..28].try_into().unwrap()),
            text as u64 + 14
        );
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
        uninstall_patch(&patch, &PLATFORM);
        assert_eq!(std::slice::from_raw_parts(text, 14), &before[..]);

        // CBaseTempEntity::Create: its first lea is rewritten in the trampoline
        let create = scan_sig(base, l::TE_CREATE_SIG).expect("Create");
        let orig15 = std::slice::from_raw_parts(create, 15).to_vec();
        let patch = install_patch_relocating(
            create,
            l::TE_CREATE_SIG,
            15,
            &[8], // the Linux build's TE_CREATE_RIP_LEAS (tests use the Windows constants)
            hook_te_create as *const () as usize,
            &PLATFORM,
        )
        .expect("install Create");
        let t = std::slice::from_raw_parts(patch.trampoline as *const u8, 18 + 14);
        assert_eq!(&t[..8], &orig15[..8]);
        assert_eq!(&t[8..10], &[0x49, 0xBE], "movabs r14");
        let disp = i32::from_le_bytes(orig15[11..15].try_into().unwrap());
        assert_eq!(
            u64::from_le_bytes(t[10..18].try_into().unwrap()),
            (create as usize + 15).wrapping_add_signed(disp as isize) as u64
        );
        assert_eq!(
            u64::from_le_bytes(t[24..32].try_into().unwrap()) as usize,
            create as usize + 15
        );
        uninstall_patch(&patch, &PLATFORM);

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

#[test]
pub(crate) fn linux_create_prologue_relocates_its_lea() {
    let Some(d) = server_so() else {
        eprintln!("SERVER_SO_X64 not set; skipping");
        return;
    };
    let at = elf_matches(&d, linux_sigs::TE_CREATE_SIG)[0];
    // single PT_LOAD at vaddr == file offset is not guaranteed: find the file offset
    let pat = parse_sig(linux_sigs::TE_CREATE_SIG);
    let off = find_all(&d, &pat)[0];
    let code = &d[off..off + 15];
    let abs = relocate_rip_lea(code, 8, at as u64).expect("lea r14");
    let disp = i32::from_le_bytes(code[11..15].try_into().unwrap());
    assert_eq!(&abs[..2], &[0x49, 0xBE]);
    assert_eq!(
        u64::from_le_bytes(abs[2..].try_into().unwrap()),
        (at + 8 + 7).wrapping_add_signed(isize::try_from(disp).unwrap()) as u64
    );
}
