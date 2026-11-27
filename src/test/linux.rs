// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67

use super::*;

pub(crate) const FUNC_LEN: usize = 0xbb;

/// A real 64-bit Linux build the tests can run against, and what is known about it. The files
/// come from the environment (a build whose variables are unset is skipped).
pub(crate) struct Build {
    pub(crate) name: &'static str,
    server_env: &'static str,
    client_env: &'static str,
    /// RVA of `CBaseEntity::SetWaterLevel`.
    pub(crate) set_water_level: usize,
    pub(crate) water_level: usize,
    pub(crate) eflags: usize,
    pub(crate) swept: usize,
    /// RVAs of the variables' loads: the variable itself, or its GOT slot (x86-64 branch).
    pub(crate) enginetrace: usize,
    pub(crate) physenv: usize,
    pub(crate) entity_array: usize,
    pub(crate) client_enginetrace: usize,
    /// Where the displacement is in the `UpdateWaterState` match.
    pub(crate) upd_disp: usize,
}

pub(crate) const BUILDS: [Build; 2] = [
    Build {
        name: "dev branch",
        server_env: "SERVER_SO_X64",
        client_env: "CLIENT_SO_X64",
        set_water_level: 0x00bd_b670,
        water_level: 0x2db,
        eflags: 0x1c0,
        swept: 0x41,
        enginetrace: 0x01b1_6f78,
        physenv: 0x01a0_da30,
        entity_array: 0x0193_b850,
        client_enginetrace: 0x0121_0fd0,
        upd_disp: 95,
    },
    Build {
        name: "x86-64 branch",
        server_env: "SERVER_SO_X86_64",
        client_env: "CLIENT_SO_X86_64",
        set_water_level: 0x0092_1590,
        water_level: 0x2d3,
        eflags: 0x1b8,
        swept: 0x49,
        enginetrace: 0x019e_17b0,
        physenv: 0x019e_1bd8,
        entity_array: 0x019e_18a0,
        client_enginetrace: 0x012c_5f88,
        upd_disp: 104,
    },
];

impl Build {
    pub(crate) fn server(&self) -> Option<Vec<u8>> {
        std::fs::read(std::env::var(self.server_env).ok()?).ok()
    }

    pub(crate) fn client(&self) -> Option<Vec<u8>> {
        std::fs::read(std::env::var(self.client_env).ok()?).ok()
    }
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
    let mut out = Vec::new();
    for i in 0..phnum {
        let ph = phoff + i * phentsize;
        if rd(ph, 4) == 1 && rd(ph + 4, 4) & 1 != 0 {
            let (off, va, filesz) = (rd(ph + 8, 8), rd(ph + 0x10, 8), rd(ph + 0x20, 8));
            out.extend(
                find_sig(&d[off..off + filesz], sig)
                    .into_iter()
                    .map(|m| va + m),
            );
        }
    }
    out
}

#[test]
pub(crate) fn linux_signatures_are_unique_in_server_so() {
    use linux_sigs as l;
    for build in &BUILDS {
        let Some(d) = build.server() else {
            eprintln!("{} not set; skipping", build.server_env);
            continue;
        };
        let mut sigs = vec![
            ("SetWaterLevel", l::SIG.to_string()),
            ("UpdateWaterState", l::UPDATE_WATER_STATE_SIG.to_string()),
            ("FluidEndTouch", l::FLUID_END_TOUCH_SIG.to_string()),
            ("UTIL_BubbleTrail", l::BUBBLE_TRAIL_SIG.to_string()),
            ("DispatchEffect", l::DISPATCH_EFFECT_SIG.to_string()),
            ("CBaseTempEntity::Create", l::TE_CREATE_SIG.to_string()),
            ("physenv user", l::PHYSENV_SIG.to_string()),
            ("ray setup", l::RAY_INIT_SIG.to_string()),
        ];
        for t in l::EXPLOSION_TARGETS {
            sigs.push(("explosion target", t.join(" | ")));
        }
        for (name, sig) in sigs {
            let m = elf_matches(&d, &sig);
            assert_eq!(m.len(), 1, "{}: {name}: {} matches", build.name, m.len());
        }
        // the entity array idiom has many sites, all naming one global
        let sites = elf_matches(&d, l::ENTITY_ARRAY_SIG);
        assert!(sites.len() > 1, "{}", build.name);
    }
}

#[test]
pub(crate) fn linux_client_trace_signature_finds_the_enginetrace_variable() {
    for build in &BUILDS {
        let Some(d) = build.client() else {
            eprintln!("{} not set; skipping", build.client_env);
            continue;
        };
        let m = elf_matches(&d, linux_sigs::CLIENT_TRACE_SIG);
        assert_eq!(m.len(), 1, "{}: {} matches", build.name, m.len());
        unsafe {
            let base = map_elf(&d);
            let at = scan_sig(base, linux_sigs::CLIENT_TRACE_SIG).expect("matches once");
            assert_eq!(at as usize - base as usize, m[0]);
            // `C_BaseEntity::UpdateWaterState` loads the `enginetrace` variable (or its GOT
            // slot), the one its `TraceRay` calls use. (The tests build with the Windows
            // constants, so the Linux signature is passed.)
            let var = last_global(&decode_match(at, linux_sigs::CLIENT_TRACE_SIG)).expect("a load");
            assert_eq!(
                var - base as usize,
                build.client_enginetrace,
                "{}",
                build.name
            );
        }
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
    for build in &BUILDS {
        match build.server() {
            Some(d) => unsafe { patches_install(build, &d) },
            None => eprintln!("{} not set; skipping", build.server_env),
        }
    }
}

unsafe fn patches_install(build: &Build, d: &[u8]) {
    use linux_sigs as l;
    unsafe {
        let base = map_elf(d);
        // the executable segments back to r-x so the real make_writable path is used
        extern "C" {
            fn mprotect(addr: *mut c_void, len: usize, prot: i32) -> i32;
        }
        let rva_of = |at: *mut u8| at as usize - base as usize;
        let text = scan_sig(base, l::SIG).expect("SetWaterLevel");
        assert_eq!(rva_of(text), build.set_water_level, "{}", build.name);
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

        // globals, decoded from the matches (the tests build with the Windows constants, so
        // the Linux signatures are passed explicitly)
        let upd = scan_sig(base, l::UPDATE_WATER_STATE_SIG).expect("UpdateWaterState");
        let enginetrace = last_global(&decode_match(upd, l::UPDATE_WATER_STATE_SIG)).unwrap();
        assert_eq!(enginetrace - base as usize, build.enginetrace);
        let env = scan_sig(base, l::PHYSENV_SIG).expect("physenv user");
        let physenv = only_global(&decode_match(env, l::PHYSENV_SIG)).unwrap();
        assert_eq!(physenv - base as usize, build.physenv);
        for at in scan_sig_all(base, l::ENTITY_ARRAY_SIG).expect("entity array idiom") {
            let array = only_global(&decode_match(at, l::ENTITY_ARRAY_SIG)).unwrap();
            assert_eq!(array - base as usize, build.entity_array);
        }

        // every explosion function and the other hooked prologues can be detoured and
        // restored (the x86-64 branch's helicopter bomb starts with a rip-relative lea)
        let mut targets: Vec<String> = l::EXPLOSION_TARGETS.iter().map(|t| t.join(" | ")).collect();
        targets.extend([
            l::BUBBLE_TRAIL_SIG.to_string(),
            l::FLUID_END_TOUCH_SIG.to_string(),
            l::DISPATCH_EFFECT_SIG.to_string(),
            l::UPDATE_WATER_STATE_SIG.to_string(),
        ]);
        for sig in targets {
            let at = scan_sig(base, &sig).expect("matches once");
            let before = std::slice::from_raw_parts(at, 32).to_vec();
            let detour = install_detour(at, hook_te_create as *const (), &slot)
                .unwrap_or_else(|why| panic!("{}: detour of {sig}: {why:?}", build.name));
            assert_ne!(std::slice::from_raw_parts(at, 5), &before[..5]);
            drop(detour);
            assert_eq!(std::slice::from_raw_parts(at, 32), &before[..]);
        }
    }
}

#[test]
pub(crate) fn lua_shared_dir_is_the_folder_holding_bin_linux64() {
    use std::path::PathBuf;
    // The 64-bit client and a listen server keep it under garrysmod/.
    assert_eq!(
        os::lua_shared_dir(
            b"/home/u/.local/share/Steam/steamapps/common/GarrysMod/garrysmod/bin/linux64/lua_shared.so"
        ),
        Some(PathBuf::from(
            "/home/u/.local/share/Steam/steamapps/common/GarrysMod/garrysmod/"
        ))
    );
    // A dedicated server's root, and a relative path, are already what `gmod` expects.
    assert_eq!(
        os::lua_shared_dir(b"/srv/gmod/bin/linux64/lua_shared.so"),
        Some(PathBuf::from("/srv/gmod/"))
    );
    assert_eq!(
        os::lua_shared_dir(b"bin/linux64/lua_shared.so"),
        Some(PathBuf::new())
    );
    // Not that library, or a name that merely ends the same way.
    assert_eq!(
        os::lua_shared_dir(b"/x/bin/linux64/lua_shared_srv.so"),
        None
    );
    assert_eq!(os::lua_shared_dir(b"/x/mybin/linux64/lua_shared.so"), None);
    assert_eq!(os::lua_shared_dir(b"/usr/lib/liblua.so"), None);
}

#[test]
pub(crate) fn a_missing_lua_shared_is_an_error_not_a_panic() {
    let _g = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let before = std::env::current_dir().unwrap();
    // The test process has no lua_shared.so loaded.
    let why = unsafe { load_lua_shared() }.unwrap_err();
    assert!(why.contains("lua_shared.so"), "{why}");
    assert_eq!(std::env::current_dir().unwrap(), before);
}

#[test]
pub(crate) fn a_dedicated_servers_lua_shared_srv_gets_the_expected_path_only_during_the_load() {
    let _g = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let before = std::env::current_dir().unwrap();
    let dir = std::env::temp_dir().join(format!("wl-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("lua_shared_srv.so");
    std::fs::write(&file, b"x").unwrap();
    let seen = unsafe {
        os::with_lua_shared_visible(file.as_os_str().as_encoded_bytes(), || {
            std::fs::canonicalize("bin/linux64/lua_shared.so").ok()
        })
    }
    .unwrap();
    assert_eq!(seen, Some(std::fs::canonicalize(&file).unwrap()));
    assert_eq!(std::env::current_dir().unwrap(), before);
    assert!(!std::env::temp_dir()
        .join(format!("waterleveler-{}", std::process::id()))
        .exists());
    // A panic inside the load is an error, and still cleans up.
    let why = unsafe {
        os::with_lua_shared_visible(file.as_os_str().as_encoded_bytes(), || -> () {
            panic!("boom")
        })
    }
    .unwrap_err();
    assert!(why.contains("lua_shared_srv.so"), "{why}");
    assert_eq!(std::env::current_dir().unwrap(), before);
    std::fs::remove_dir_all(dir).unwrap();
}
