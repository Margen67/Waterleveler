// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67
// Exact float comparisons are on values the code copies or sets, items sit next to the
// test that uses them.
#![allow(clippy::float_cmp, clippy::items_after_statements)]
use super::engine_hook::*;
use super::*;
use std::sync::atomic::Ordering;

const FUNC_LEN: usize = 0xbb;

extern "C" {
    fn mmap(
        addr: *mut c_void,
        len: usize,
        prot: c_int,
        flags: c_int,
        fd: c_int,
        off: i64,
    ) -> *mut c_void;
}

fn server_dll() -> Option<Vec<u8>> {
    let path = std::env::var("SERVER_DLL_X64").ok()?;
    std::fs::read(path).ok()
}

fn server_so() -> Option<Vec<u8>> {
    let path = std::env::var("SERVER_SO_X64").ok()?;
    std::fs::read(path).ok()
}

/// Virtual addresses of every match of `sig` in the executable segments of a 64-bit ELF.
fn elf_matches(d: &[u8], sig: &str) -> Vec<usize> {
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
fn relocated_rip_lea_loads_the_same_address() {
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
fn linux_signatures_are_unique_in_server_so() {
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

fn client_so() -> Option<Vec<u8>> {
    let path = std::env::var("CLIENT_SO_X64").ok()?;
    std::fs::read(path).ok()
}

#[test]
fn linux_client_trace_signature_finds_the_enginetrace_variable() {
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
unsafe fn map_elf(d: &[u8]) -> *mut u8 {
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
fn linux_patches_install_on_the_mapped_server_so() {
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
            &lin::PLATFORM,
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
        uninstall_patch(&patch, &lin::PLATFORM);
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
            &lin::PLATFORM,
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
        uninstall_patch(&patch, &lin::PLATFORM);

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
fn linux_create_prologue_relocates_its_lea() {
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

fn client_dll() -> Option<Vec<u8>> {
    let path = std::env::var("CLIENT_DLL_X64").ok()?;
    std::fs::read(path).ok()
}

#[test]
fn client_trace_global_is_read_from_the_load_instruction() {
    // mov rcx,[rip+0x100]; the variable is at (instruction + 7 + 0x100).
    let mut code = vec![0x48u8, 0x8B, 0x0D, 0x00, 0x01, 0x00, 0x00, 0x90];
    code.extend_from_slice(&[0; 16]);
    let at = code.as_ptr();
    let g = unsafe { trace_global_from_load(at) };
    assert_eq!(g as usize, at as usize + 7 + 0x100);
}

#[test]
fn client_trace_signature_and_ray_helper_match_once_in_client_dll() {
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

struct Pe {
    size_of_image: usize,
    size_of_headers: usize,
    /// (name, rva, vsize, raw offset, raw size, characteristics)
    sections: Vec<(String, usize, usize, usize, usize, u32)>,
}

fn parse_pe(d: &[u8]) -> Pe {
    let u32at = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap()) as usize;
    let u16at = |o: usize| u16::from_le_bytes(d[o..o + 2].try_into().unwrap()) as usize;
    let nt = u32at(0x3C);
    assert_eq!(&d[nt..nt + 4], b"PE\0\0");
    assert_eq!(u16at(nt + 24), 0x20B, "expected a PE32+ image");
    let nsec = u16at(nt + 6);
    let optsz = u16at(nt + 20);
    let opt = nt + 24;
    let mut sections = Vec::new();
    for i in 0..nsec {
        let s = opt + optsz + i * 40;
        let name = String::from_utf8_lossy(&d[s..s + 8])
            .trim_end_matches('\0')
            .to_string();
        sections.push((
            name,
            u32at(s + 12),
            u32at(s + 8),
            u32at(s + 20),
            u32at(s + 16),
            u32at(s + 36) as u32,
        ));
    }
    Pe {
        size_of_image: u32at(opt + 56),
        size_of_headers: u32at(opt + 60),
        sections,
    }
}

/// Lays the file out the way the Windows loader would (headers + sections).
fn map_image(d: &[u8], pe: &Pe) -> Vec<u8> {
    let mut img = vec![0u8; pe.size_of_image];
    img[..pe.size_of_headers].copy_from_slice(&d[..pe.size_of_headers]);
    for (_, rva, _, raw, rawsz, _) in &pe.sections {
        img[*rva..*rva + *rawsz].copy_from_slice(&d[*raw..*raw + *rawsz]);
    }
    img
}

unsafe fn rwx(len: usize) -> *mut u8 {
    unsafe {
        // PROT_READ|WRITE|EXEC, MAP_PRIVATE|MAP_ANONYMOUS
        let p = mmap(std::ptr::null_mut(), len, 7, 0x22, -1, 0);
        assert_ne!(p as isize, -1, "mmap failed");
        p.cast::<u8>()
    }
}
unsafe fn t_alloc(n: usize) -> *mut u8 {
    unsafe { rwx(n.max(4096)) }
}
// Same signature as the Windows implementation it stands in for.
#[allow(clippy::unnecessary_wraps)]
const unsafe fn t_writable(_: *mut u8, _: usize) -> Option<u32> {
    Some(0)
}
const unsafe fn t_restore(_: *mut u8, _: usize, _: u32) {}
const unsafe fn t_flush(_: *mut u8, _: usize) {}
const TEST_PLATFORM: Platform = Platform {
    alloc_exec: t_alloc,
    make_writable: t_writable,
    restore_protect: t_restore,
    flush: t_flush,
};

#[test]
fn detour_and_trampoline_layout() {
    let d = build_detour(0x1122_3344_5566_7788);
    assert_eq!(&d[..6], &[0xFF, 0x25, 0, 0, 0, 0]);
    assert_eq!(&d[6..14], &0x1122_3344_5566_7788u64.to_le_bytes());
    assert_eq!(d[14], 0x90);
    let stolen = [0xAAu8; STOLEN];
    let t = build_trampoline(&stolen, 0x0102_0304_0506_0708);
    assert_eq!(&t[..STOLEN], &stolen);
    assert_eq!(&t[STOLEN..STOLEN + 6], &[0xFF, 0x25, 0, 0, 0, 0]);
    assert_eq!(&t[STOLEN + 6..], &0x0102_0304_0506_0708u64.to_le_bytes());
}

#[test]
fn signature_matcher_handles_wildcards() {
    let pat = parse_sig("01 ?? 03");
    assert_eq!(find_all(&[9, 1, 7, 3, 1, 8, 3, 1], &pat), vec![1, 4]);
    assert!(find_all(&[1, 2], &pat).is_empty());
}

/// RVA of the one place SIG matches in the file's executable sections.
fn sig_rva(d: &[u8], pe: &Pe) -> usize {
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
fn signature_is_unique_in_server_dll() {
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
fn scan_image_finds_the_function() {
    let Some(d) = server_dll() else { return };
    let pe = parse_pe(&d);
    let img = map_image(&d, &pe);
    let found = unsafe { scan_image(img.as_ptr()) }.expect("scan failed");
    assert_eq!(found as usize - img.as_ptr() as usize, sig_rva(&d, &pe));
}

#[test]
fn hook_substitutes_override_on_real_function_bytes() {
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
        let wl = read_disp(target, WATER_LEVEL_DISP_AT[0]);
        assert!(WATER_LEVEL_DISP_AT
            .iter()
            .all(|&i| read_disp(target, i) == wl));
        let original = install(target, &TEST_PLATFORM).expect("install failed");
        assert_eq!(&original[..5], &[0x48, 0x89, 0x5C, 0x24, 0x08]);
        // The entry now starts with the detour.
        assert_eq!(std::slice::from_raw_parts(target, 2), &[0xFF, 0x25]);

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
        overrides().insert(5, 1);
        set_water_level(this, 3);
        assert_eq!(entity[wl], 1);
        assert_eq!(HITS.load(Ordering::Relaxed), hits_before + 1);

        // 2) No override: the call goes through the trampoline, which runs
        //    the relocated prologue and continues in the real function.
        //    level == current again so it returns without side effects.
        overrides().remove(&5);
        set_water_level(this, 1);
        assert_eq!(entity[wl], 1);
        assert_eq!(HITS.load(Ordering::Relaxed), hits_before + 1);

        // 3) Entities without a pev are passed through untouched.
        let mut bare = vec![0u8; 0x400];
        bare[wl] = 2;
        set_water_level(bare.as_mut_ptr().cast::<c_void>(), 2);
        assert_eq!(bare[wl], 2);

        // 4) Uninstall restores the original bytes exactly.
        uninstall(target, &original, &TEST_PLATFORM);
        assert_eq!(std::slice::from_raw_parts(target, FUNC_LEN), func);
    }
}

#[test]
fn volume_math() {
    let v = [Volume {
        id: 1,
        min: [0.0, 0.0, 0.0],
        max: [128.0, 128.0, 128.0],
        contents: CONTENTS_WATER,
    }];
    assert!(point_in_volumes(&v, [64.0, 64.0, 64.0]));
    assert!(point_in_volumes(&v, [0.0, 128.0, 0.0])); // edges are inside
    assert!(!point_in_volumes(&v, [129.0, 64.0, 64.0]));
    assert!(!point_in_volumes(&v, [64.0, 64.0, -0.5]));
    assert!(!point_in_volumes(&[], [0.0, 0.0, 0.0]));
}

#[test]
fn enginetrace_pointer_resolves_in_server_dll() {
    let Some(d) = server_dll() else { return };
    let pe = parse_pe(&d);
    let img = map_image(&d, &pe);
    unsafe {
        let func = scan_sig(img.as_ptr(), UPDATE_WATER_STATE_SIG).expect("scan failed");
        // RVAs move between game updates; only the structure is checked.
        let global = trace_global_from(func);
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

static GPC_ARGS: Mutex<(usize, usize)> = Mutex::new((0, 0));

/// Stand-in for the engine's `GetPointContents`: reports `CONTENTS_SOLID`.
unsafe extern "win64" fn fake_get_point_contents(
    _this: *mut c_void,
    _pos: *const f32,
    a3: usize,
    a4: usize,
) -> c_int {
    *GPC_ARGS.lock().unwrap() = (a3, a4);
    0x1
}

/// Tests that touch the shared VOLUMES / hook statics must not interleave.
static TEST_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn rays_starting_inside_behave_like_a_real_water_brush() {
    let _g = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    unsafe {
        let vols = cube();

        // The engine's water-height probe: starts 256 above a point that is
        // inside, ends at that point (also inside) -> allsolid, fraction 0,
        // end point = start.
        let ray = make_ray(
            [64.0, 64.0, 100.0],
            [0.0, 0.0, -40.0],
            [0.0; 3],
            [0.0; 3],
            true,
        );
        let mut tr = miss_trace();
        assert!(apply_water_to_trace(
            &vols,
            ray.as_ptr(),
            tr.as_mut_ptr(),
            0x1234
        ));
        assert_eq!(f32at(&tr, 0x2c), 0.0, "fraction");
        assert_eq!(f32at(&tr, 0x0c + 8), 100.0, "endpos = start");
        assert_eq!((tr[0x36], tr[0x37]), (1, 1), "allsolid / startsolid");
        assert_eq!(
            i32::from_le_bytes(tr[0x30..0x34].try_into().unwrap()),
            CONTENTS_WATER
        );
        assert_eq!(
            u64::from_le_bytes(tr[0x58..0x60].try_into().unwrap()),
            0x1234,
            "m_pEnt = the world entity"
        );

        // Starts inside but leaves: startsolid and fractionleftsolid, the rest is the engine's.
        let ray = make_ray(
            [64.0, 64.0, 64.0],
            [0.0, 0.0, 300.0],
            [0.0; 3],
            [0.0; 3],
            true,
        );
        let mut tr = miss_trace();
        let before = tr;
        assert!(apply_water_to_trace(
            &vols,
            ray.as_ptr(),
            tr.as_mut_ptr(),
            0x1234
        ));
        assert_eq!((tr[0x36], tr[0x37]), (0, 1));
        let mut expect = before;
        expect[0x37] = 1;
        expect[0x38..0x3c].copy_from_slice(&(64.0f32 / 300.0).to_le_bytes());
        assert_eq!(tr, expect);
    }
}

use std::sync::atomic::AtomicUsize;
static END_TOUCH_ORIGINAL_CALLS: AtomicUsize = AtomicUsize::new(0);
static UNTOUCH_EVENTS: AtomicUsize = AtomicUsize::new(0);
static UNTOUCH_CONTENTS: AtomicUsize = AtomicUsize::new(0);

unsafe extern "win64" fn fake_end_touch(_: *mut c_void, _: *mut c_void, _: *mut c_void) {
    END_TOUCH_ORIGINAL_CALLS.fetch_add(1, Ordering::SeqCst);
}
unsafe extern "win64" fn fake_get_game_data(object: *mut c_void) -> *mut u8 {
    unsafe { *(object as *const *mut u8).add(1) }
}
unsafe extern "win64" fn fake_on_entity_event(_: *mut u8, event: c_int, data: *mut c_void) {
    assert_eq!(event, ENTITY_EVENT_WATER_UNTOUCH);
    UNTOUCH_EVENTS.fetch_add(1, Ordering::SeqCst);
    UNTOUCH_CONTENTS.store(data as usize, Ordering::SeqCst);
}

#[test]
fn end_touch_with_a_destroyed_fluid_skips_the_splash_code() {
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
        .copy_from_slice(&(EFL_TOUCHING_FLUID | 0x4).to_le_bytes());
    let mut object_vtable = [0usize; OBJ_GET_GAME_DATA_SLOT + 1];
    object_vtable[OBJ_GET_GAME_DATA_SLOT] = fake_get_game_data as *const () as usize;
    let object = [
        object_vtable.as_ptr() as usize,
        entity.as_mut_ptr() as usize,
    ];
    let live = 0x1000usize;
    let dead = 0x2000usize;
    fluid_died(dead, CONTENTS_SLIME);
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
        CONTENTS_SLIME as usize
    );
    let flags = u32::from_le_bytes(
        entity[ENTITY_EFLAGS_OFFSET..ENTITY_EFLAGS_OFFSET + 4]
            .try_into()
            .unwrap(),
    );
    assert_eq!(flags, 0x4, "only EFL_TOUCHING_FLUID is cleared");
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
fn fluid_end_touch_signature_is_unique_in_server_dll() {
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
    // The stolen bytes end on an instruction boundary (`inc dword [rcx+0x1f0]`).
    let o = text_raw + hits[0] + FLUID_END_TOUCH_STOLEN;
    assert_eq!(&d[o..o + 3], &[0xFF, 0x81, 0xF0]);
}

#[test]
fn bubble_trail_signature_is_unique_in_server_dll() {
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
fn bubble_trail_clip_math() {
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

type BubbleCall = ([f32; 3], [f32; 3], i32);
static BUBBLE_CALLS: Mutex<Vec<BubbleCall>> = Mutex::new(Vec::new());

unsafe extern "win64" fn fake_bubble_trail(from: *const f32, to: *const f32, count: c_int) {
    unsafe {
        BUBBLE_CALLS.lock().unwrap().push((
            [*from, *from.add(1), *from.add(2)],
            [*to, *to.add(1), *to.add(2)],
            count,
        ));
    }
}

#[test]
fn bubble_hook_clips_count_and_releases_the_lock() {
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
fn install_patch_on_real_bubble_trail_bytes() {
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

#[test]
fn fluid_params_layout_matches_the_engine() {
    use std::mem::{offset_of, size_of};
    assert_eq!(offset_of!(FluidParams, surface_plane), 0x00);
    assert_eq!(offset_of!(FluidParams, current_velocity), 0x10);
    assert_eq!(offset_of!(FluidParams, damping), 0x1c); // vphysics reads [params+0x1c]
    assert_eq!(offset_of!(FluidParams, game_data), 0x28); // [params+0x28]
    assert_eq!(offset_of!(FluidParams, contents), 0x34); // [params+0x34]
    assert_eq!(size_of::<FluidParams>(), 0x38);
}

#[test]
fn physenv_signature_is_unique_and_points_into_data() {
    let Some(d) = server_dll() else {
        eprintln!("SERVER_DLL_X64 not set; skipping");
        return;
    };
    let pe = parse_pe(&d);
    let img = map_image(&d, &pe);
    unsafe {
        let at = scan_sig(img.as_ptr(), PHYSENV_SIG).expect("signature must match exactly once");
        let global = physenv_global_from(at);
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

type FluidCall = (&'static str, usize, usize, [f32; 4], f32, i32);
static FLUID_CALLS: Mutex<Vec<FluidCall>> = Mutex::new(Vec::new());

unsafe extern "win64" fn fake_create(
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
unsafe extern "win64" fn fake_destroy(env: *mut c_void, fluid: *mut c_void) {
    FLUID_CALLS
        .lock()
        .unwrap()
        .push(("destroy", env as usize, fluid as usize, [0.0; 4], 0.0, 0));
}

static FLAGS: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(0x0001);
unsafe extern "win64" fn fake_get_flags(_: *mut c_void) -> u16 {
    FLAGS.load(Ordering::SeqCst)
}
unsafe extern "win64" fn fake_set_flags(_: *mut c_void, f: u16) {
    FLAGS.store(f, Ordering::SeqCst);
}

#[test]
fn te_explosion_vtable_is_found_and_slot_4_is_create() {
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
fn water_explosion_effect_data_matches_what_the_engine_builds() {
    let d = water_explosion_effect_data([1.0, 2.0, 3.0]);
    assert_eq!(f32::from_le_bytes(d[8..12].try_into().unwrap()), 3.0);
    assert_eq!(u32::from_le_bytes(d[0x30..0x34].try_into().unwrap()), 0x428);
    assert_eq!(f32::from_le_bytes(d[0x38..0x3c].try_into().unwrap()), 128.0);
    assert_eq!(d[0x88], 1);
}

#[test]
fn explosion_in_a_volume_is_redirected_outside_is_not() {
    let _g = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    volumes().clear();
    volumes().push(Volume {
        id: 1,
        min: [0.0; 3],
        max: [100.0; 3],
        contents: CONTENTS_WATER,
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
fn redirected_explosion_carries_the_blast_point_in_the_effect_start() {
    let _g = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    volumes().clear();
    volumes().push(Volume {
        id: 1,
        min: [0.0; 3],
        max: [1000.0; 3],
        contents: CONTENTS_WATER,
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
fn slime_volumes_report_slime_and_only_water_is_a_surface() {
    let vols = [
        Volume {
            id: 1,
            min: [0.0; 3],
            max: [100.0; 3],
            contents: CONTENTS_SLIME,
        },
        Volume {
            id: 2,
            min: [200.0, 0.0, 0.0],
            max: [300.0, 100.0, 100.0],
            contents: CONTENTS_WATER,
        },
    ];
    assert_eq!(contents_at(&vols, [50.0; 3]), CONTENTS_SLIME);
    assert_eq!(contents_at(&vols, [250.0, 50.0, 50.0]), CONTENTS_WATER);
    assert_eq!(contents_at(&vols, [150.0; 3]), 0);
    assert_eq!(surface_z_at(&vols, [50.0; 3]), None);
    assert_eq!(surface_z_at(&vols, [250.0, 50.0, 50.0]), Some(100.0));
    let both = [
        vols[0],
        Volume { id: 3, ..vols[0] },
        Volume {
            id: 4,
            contents: CONTENTS_WATER,
            ..vols[0]
        },
    ];
    assert_eq!(
        contents_at(&both, [50.0; 3]),
        CONTENTS_SLIME | CONTENTS_WATER
    );
}

#[test]
fn fluid_defaults_match_a_map_water_brush() {
    let d = FluidSettings::default();
    assert_eq!(
        (d.torque, d.viscosity, d.aerodynamics, d.current),
        (0.0, 0.0, false, [0.0; 3])
    );
    assert_eq!(d.damping, 0.01);
    assert_eq!(d.contents, CONTENTS_WATER);
}

#[test]
fn te_create_signature_is_unique_and_stolen_bytes_are_whole_instructions() {
    let Some(d) = server_dll() else {
        eprintln!("SERVER_DLL_X64 not set; skipping");
        return;
    };
    let pe = parse_pe(&d);
    let img = map_image(&d, &pe);
    unsafe {
        let at = scan_sig(img.as_ptr(), TE_CREATE_SIG).expect("must match once");
        let b = std::slice::from_raw_parts(at, TE_CREATE_STOLEN + 7);
        // The instruction after the stolen bytes is the rip-relative `mov rax, [rip+x]`.
        assert_eq!(
            &b[TE_CREATE_STOLEN..TE_CREATE_STOLEN + 3],
            &[0x48, 0x8B, 0x05]
        );
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
fn dispatch_effect_signature_is_unique() {
    let Some(d) = server_dll() else {
        eprintln!("SERVER_DLL_X64 not set; skipping");
        return;
    };
    let pe = parse_pe(&d);
    let img = map_image(&d, &pe);
    unsafe {
        let at = scan_sig(img.as_ptr(), DISPATCH_EFFECT_SIG).expect("must match once");
        let b = std::slice::from_raw_parts(at, DISPATCH_EFFECT_STOLEN + 5);
        assert_eq!(b[DISPATCH_EFFECT_STOLEN], 0x48); // next instruction starts here (lea rcx,[rsp+30h])
    }
}

#[test]
fn entity_array_global_is_found_and_agrees() {
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
fn world_entity_is_entry_zero_of_the_array() {
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
fn water_hit_names_the_world_entity_or_is_skipped() {
    let _g = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let vols = [Volume {
        id: 1,
        min: [0.0; 3],
        max: [100.0; 3],
        contents: CONTENTS_WATER,
    }];
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
    unsafe {
        // No entity known anywhere: leave the trace alone.
        assert!(!apply_water_to_trace(
            &vols,
            ray.as_ptr(),
            tr.as_mut_ptr(),
            0
        ));
        assert_eq!(tr[0x58..0x60], [0u8; 8]);
        // World known: the hit names it.
        assert!(apply_water_to_trace(
            &vols,
            ray.as_ptr(),
            tr.as_mut_ptr(),
            0x1234
        ));
        assert_eq!(
            usize::from_le_bytes(tr[0x58..0x60].try_into().unwrap()),
            0x1234
        );
    }
}

#[test]
fn explosion_signatures_are_unique_and_stolen_bytes_are_whole_instructions() {
    let Some(d) = server_dll() else {
        eprintln!("SERVER_DLL_X64 not set; skipping");
        return;
    };
    let pe = parse_pe(&d);
    let img = map_image(&d, &pe);
    for i in 0..EXPLOSION_TARGETS.len() {
        unsafe {
            let (at, _sig, stolen) = find_explosion_target(img.as_ptr(), i)
                .expect("explosion signature must match exactly once");
            assert!(stolen >= 14);
            let bytes = std::slice::from_raw_parts(at, stolen + 8);
            // No rip-relative operands or relative branches in the stolen part.
            assert!(
                !bytes[..stolen]
                    .windows(1)
                    .any(|b| b[0] == 0xE8 || b[0] == 0xE9),
                "call/jmp in stolen bytes"
            );
        }
    }
}

#[test]
fn explosion_hook_raises_water_level_only_inside_a_volume() {
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
        contents: CONTENTS_WATER,
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

#[test]
fn fluid_object_gets_callback_flag_via_slots_0xb0_0xb8() {
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
fn fluid_calls_use_vtable_slots_11_and_12() {
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
            contents: CONTENTS_WATER,
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
                CONTENTS_WATER
            )
        );
        assert_eq!(calls[1].0, "destroy");
        assert_eq!(calls[1].2, 0x7777);
        drop(calls);
    }
}

fn make_ray(
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

fn miss_trace() -> [u8; 0x70] {
    let mut t = [0u8; 0x70];
    t[0x2c..0x30].copy_from_slice(&1.0f32.to_le_bytes()); // fraction = 1
    t[0x58..0x60].copy_from_slice(&0xABCDu64.to_le_bytes()); // m_pEnt sentinel
    t
}

fn f32at(b: &[u8], o: usize) -> f32 {
    f32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

const fn cube() -> [Volume; 1] {
    [Volume {
        id: 1,
        min: [0.0; 3],
        max: [128.0; 3],
        contents: CONTENTS_WATER,
    }]
}

#[test]
fn trace_mask_selects_the_volumes_it_asks_for() {
    unsafe {
        // A water volume and a slime volume side by side along the ray.
        let vols = [
            cube()[0],
            Volume {
                id: 2,
                min: [200.0, 0.0, 0.0],
                max: [328.0, 128.0, 128.0],
                contents: CONTENTS_SLIME,
            },
        ];
        let ray = make_ray(
            [264.0, 64.0, 300.0],
            [0.0, 0.0, -300.0],
            [0.0; 3],
            [0.0; 3],
            true,
        );
        // Only the slime volume is under the ray: a water-only mask must leave it alone.
        let mut tr = miss_trace();
        assert!(!apply_water_to_trace_masked(
            &vols,
            ray.as_ptr(),
            tr.as_mut_ptr(),
            0x1234,
            CONTENTS_WATER as u32
        ));
        assert_eq!(tr, miss_trace());
        // A slime mask, and a mask with both, see it.
        for mask in [
            CONTENTS_SLIME as u32,
            (CONTENTS_WATER | CONTENTS_SLIME) as u32,
        ] {
            let mut tr = miss_trace();
            assert!(apply_water_to_trace_masked(
                &vols,
                ray.as_ptr(),
                tr.as_mut_ptr(),
                0x1234,
                mask
            ));
            assert_eq!(
                i32::from_le_bytes(tr[0x30..0x34].try_into().unwrap()),
                CONTENTS_SLIME
            );
        }
    }
}

#[test]
fn volume_bounds_follow_every_change_and_never_reject_a_point_inside() {
    let a = cube()[0];
    let b = Volume {
        id: 2,
        min: [1000.0, -50.0, 20.0],
        max: [1100.0, 50.0, 60.0],
        contents: CONTENTS_SLIME,
    };
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
                let inside = contents_at(&list, p) != 0;
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
fn rays_that_miss_the_bounds_never_touch_a_volume() {
    unsafe {
        let mut list = VolumeList::new();
        assert!(ray_misses_bounds(
            list.bounds,
            make_ray([0.0; 3], [0.0, 0.0, -10.0], [0.0; 3], [0.0; 3], true).as_ptr()
        ));
        list.push(cube()[0]);
        list.push(Volume {
            id: 2,
            min: [1000.0, -50.0, 20.0],
            max: [1100.0, 50.0, 60.0],
            contents: CONTENTS_SLIME,
        });
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
fn override_count_follows_the_overrides_map() {
    let _g = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    overrides().clear();
    assert_eq!(OVERRIDE_COUNT.load(Ordering::Relaxed), 0);
    overrides().insert(5, 1);
    overrides().insert(9, 2);
    assert_eq!(OVERRIDE_COUNT.load(Ordering::Relaxed), 2);
    overrides().remove(&5);
    assert_eq!(OVERRIDE_COUNT.load(Ordering::Relaxed), 1);
    overrides().clear();
    assert_eq!(OVERRIDE_COUNT.load(Ordering::Relaxed), 0);
}

#[test]
fn segment_box_entry_math() {
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
fn newer_ray_layout_keeps_the_swept_flag_at_0x49() {
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
            u32::MAX
        ));
        // Reading the old position (0x41, zero here) would call it "not swept".
        let mut tr = miss_trace();
        assert!(!apply_water_to_trace_at(
            &vols,
            ray.as_ptr(),
            tr.as_mut_ptr(),
            0x1234,
            0x41,
            u32::MAX
        ));
    }
}

#[test]
fn ray_helper_signature_gives_the_swept_offset() {
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
fn hull_pellet_line_reaches_the_floor_inside_the_volume() {
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
fn trace_is_rewritten_as_water_surface_hit() {
    unsafe {
        let vols = cube();

        // A line trace that would otherwise travel the full length.
        let ray = make_ray(
            [64.0, 64.0, 300.0],
            [0.0, 0.0, -300.0],
            [0.0; 3],
            [0.0; 3],
            true,
        );
        let mut tr = miss_trace();
        assert!(apply_water_to_trace(
            &vols,
            ray.as_ptr(),
            tr.as_mut_ptr(),
            0x1234
        ));
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
            CONTENTS_WATER
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
        let ray = make_ray(
            [64.0, 64.0, -100.0],
            [0.0, 0.0, 300.0],
            [0.0; 3],
            [0.0; 3],
            true,
        );
        let mut tr = miss_trace();
        assert!(apply_water_to_trace(
            &vols,
            ray.as_ptr(),
            tr.as_mut_ptr(),
            0x1234
        ));
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
        assert!(apply_water_to_trace(
            &vols,
            ray.as_ptr(),
            tr.as_mut_ptr(),
            0x1234
        ));
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
        let ray = make_ray(
            [64.0, 64.0, 300.0],
            [0.0, 0.0, -300.0],
            [0.0; 3],
            [0.0; 3],
            true,
        );
        let mut tr = miss_trace();
        tr[0x2c..0x30].copy_from_slice(&0.3f32.to_le_bytes());
        let before = tr;
        assert!(!apply_water_to_trace(
            &vols,
            ray.as_ptr(),
            tr.as_mut_ptr(),
            0x1234
        ));
        assert_eq!(tr, before);

        // Not swept, or misses the box: unchanged.
        for ray in [
            make_ray(
                [64.0, 64.0, 300.0],
                [0.0, 0.0, -300.0],
                [0.0; 3],
                [0.0; 3],
                false,
            ),
            make_ray(
                [900.0, 64.0, 300.0],
                [0.0, 0.0, -300.0],
                [0.0; 3],
                [0.0; 3],
                true,
            ),
        ] {
            let mut tr = miss_trace();
            let before = tr;
            assert!(!apply_water_to_trace(
                &vols,
                ray.as_ptr(),
                tr.as_mut_ptr(),
                0x1234
            ));
            assert_eq!(tr, before);
        }
    }
}

static TRACE_ARGS: Mutex<Vec<(u32, usize, usize, usize)>> = Mutex::new(Vec::new());

/// Stand-in for the engine's `TraceRay`: records its arguments, reports a miss.
unsafe extern "win64" fn fake_trace_ray(
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
fn trace_hook_through_vtable() {
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

        let ray = make_ray(
            [64.0, 64.0, 300.0],
            [0.0, 0.0, -300.0],
            [0.0; 3],
            [0.0; 3],
            true,
        );
        let call = |mask: u32, tr: &mut [u8; 0x70]| {
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
        call(CONTENTS_WATER as u32 | 0x10, &mut tr);
        assert!((f32at(&tr, 0x2c) - (172.0 + ENTRY_DEPTH) / 300.0).abs() < 1e-6);
        assert_eq!(
            i32::from_le_bytes(tr[0x30..0x34].try_into().unwrap()),
            CONTENTS_WATER
        );

        // No water in the mask (e.g. a solid-only trace): untouched.
        let mut tr = miss_trace();
        call(0x1, &mut tr);
        assert_eq!(f32at(&tr, 0x2c), 1.0);

        // No volumes: pure pass-through even with water in the mask.
        volumes().clear();
        let mut tr = miss_trace();
        call(CONTENTS_WATER as u32, &mut tr);
        assert_eq!(f32at(&tr, 0x2c), 1.0);

        // The original saw every argument, including the stack-passed 5th.
        let seen = TRACE_ARGS.lock().unwrap().clone();
        assert_eq!(seen.len(), 3);
        for (_, ray_ptr, filter, trace_ptr) in &seen {
            assert_eq!(*ray_ptr, ray.as_ptr() as usize);
            assert_eq!(*filter, 0x1234);
            assert_ne!(*trace_ptr, 0);
        }
        assert_eq!(seen[0].0, CONTENTS_WATER as u32 | 0x10);
        assert_eq!(TRACE_HITS.load(Ordering::Relaxed), hits_before + 1);

        uninstall_slot_hook(entry, hook, original, &TEST_PLATFORM);
        assert_eq!(
            vtable[TRACE_RAY_SLOT], original,
            "uninstall restores the slot"
        );
    }
}

#[test]
fn lane_scan_agrees_with_the_plain_scan() {
    let mut state = 0x2545_F491_u32;
    let mut next = move || {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (state >> 8) as f32 / 16_777_216.0
    };
    let mut list = VolumeList::new();
    for id in 0..40 {
        let lo = [next() * 800.0, next() * 800.0, next() * 800.0];
        let size = [next() * 200.0, next() * 200.0, next() * 200.0];
        list.push(Volume {
            id,
            min: lo,
            max: [lo[0] + size[0], lo[1] + size[1], lo[2] + size[2]],
            contents: if id % 3 == 0 {
                CONTENTS_SLIME
            } else {
                CONTENTS_WATER
            },
        });
    }
    let mut hits = 0;
    for _ in 0..20_000 {
        let p = [next() * 1000.0, next() * 1000.0, next() * 1000.0];
        let expected = contents_at(&list, p);
        hits += usize::from(expected != 0);
        assert_eq!(list.contents_at(p), expected, "{p:?}");
    }
    assert!(hits > 0);
    // Edges are inside, NaN is not.
    let v = list[0];
    assert_eq!(list.contents_at(v.min) & v.contents, v.contents);
    assert_eq!(list.contents_at([f32::NAN, 1.0, 1.0]), 0);
    list.retain(|v| v.id == 1);
    assert_eq!(list.contents_at(list[0].min), list[0].contents);
    list.clear();
    assert_eq!(list.contents_at([0.0; 3]), 0);
}

#[test]
fn every_point_query_agrees_with_the_plain_scan_at_every_size() {
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
            list.push(Volume {
                id: id as u32,
                min: lo,
                max: [lo[0] + size[0], lo[1] + size[1], lo[2] + size[2]],
                contents: if id % 4 == 0 {
                    CONTENTS_SLIME
                } else {
                    CONTENTS_WATER
                },
            });
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
fn trails_that_miss_the_bounds_are_never_clipped() {
    let mut state = 0x1234_5678_u32;
    let mut next = move || {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (state >> 8) as f32 / 16_777_216.0
    };
    let mut list = VolumeList::new();
    for id in 0..6 {
        let lo = [next() * 500.0, next() * 500.0, next() * 500.0];
        list.push(Volume {
            id,
            min: lo,
            max: [lo[0] + 90.0, lo[1] + 90.0, lo[2] + 90.0],
            contents: CONTENTS_WATER,
        });
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
fn hooks_read_the_snapshot_published_by_every_edit() {
    let _g = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    volumes().clear();
    assert!(snapshot().is_none());
    volumes().push(Volume {
        id: 1,
        min: [0.0; 3],
        max: [10.0; 3],
        contents: CONTENTS_WATER,
    });
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
fn contents_hook_adds_water_inside_volumes() {
    let _g = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    unsafe {
        // A fake IEngineTrace: object -> vtable -> [GetPointContents, ...]
        let mut vtable: Vec<usize> = vec![fake_get_point_contents as *const () as usize; 8];
        let mut object: [usize; 2] = [vtable.as_mut_ptr() as usize, 0];
        let trace = object.as_mut_ptr().cast::<c_void>();

        volumes().clear();
        volumes().push(Volume {
            id: 1,
            min: [0.0, 0.0, 0.0],
            max: [128.0, 128.0, 128.0],
            contents: CONTENTS_WATER,
        });
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
        assert_eq!(call([64.0, 64.0, 64.0]), 0x1 | CONTENTS_WATER); // inside: water added
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
