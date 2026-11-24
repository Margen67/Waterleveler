// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67
// The Windows build (and the client) needs more of the parent's names than the Linux server uses.
#[allow(clippy::wildcard_imports)]
use super::*;
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

// The engine's calling convention: Windows x64 for the Windows server, System V for
// the Linux one. Under `cfg(test)` it is always Windows x64: the tests run the
// machine code of a Windows server.dll on whatever host runs them.
#[cfg(any(windows, test))]
macro_rules! engine_fn {
    ($($t:tt)*) => { unsafe extern "win64" fn $($t)* };
}
#[cfg(not(any(windows, test)))]
macro_rules! engine_fn {
    ($($t:tt)*) => { unsafe extern "C" fn $($t)* };
}
/// Defines an `unsafe fn` with the engine's calling convention.
macro_rules! engine_abi_item {
    ($(#[$m:meta])* $v:vis unsafe fn $name:ident ( $($args:tt)* ) $(-> $ret:ty)? $body:block) => {
        #[cfg(any(windows, test))]
        $(#[$m])* $v unsafe extern "win64" fn $name($($args)*) $(-> $ret)? $body
        #[cfg(not(any(windows, test)))]
        $(#[$m])* $v unsafe extern "C" fn $name($($args)*) $(-> $ret)? $body
    };
}

/// The 64-bit Linux server's signatures. Always compiled (the tests check them against a
/// real `server_srv.so`), used only by the Linux build.
#[cfg(any(feature = "server", feature = "client"))]
#[allow(dead_code)]
pub(crate) mod linux_sigs {
    /// `client.so`: `C_BaseEntity::UpdateWaterState` (the `enginetrace` variable load).
    pub(crate) const CLIENT_TRACE_SIG: &str =
        "4C 8D 2D ?? ?? ?? ?? 31 D2 4C 89 E6 49 8B 7D 00 48 8B 07 FF 10 A9 30 40 00 00";
    pub(crate) const SIG: &str =
        "55 48 89 E5 41 56 41 55 41 54 41 89 F4 53 44 0F B6 AF D3 02 00 00 \
                       48 89 FB 41 38 F5 74 17 48 8B 07 48 8D B7 D3 02 00 00 FF 90 ?? ?? 00 00 \
                       44 88 A3 D3 02 00 00";
    pub(crate) const UPDATE_WATER_STATE_SIG: &str = "\
    55 48 89 E5 41 55 41 54 4C 8D 65 C0 53 48 8D 75 D0 4C 89 E2 48 89 FB 48 83 EC 28 \
    48 8B 05 ?? ?? ?? ?? 48 81 C7 10 02 00 00 C7 45 C8 00 00 00 00 48 C7 45 C0 00 00 00 00 \
    C7 45 D8 00 00 00 00 48 89 45 D0 E8 ?? ?? ?? ?? 31 F6 48 89 DF E8 ?? ?? ?? ?? 31 F6 \
    48 89 DF E8 ?? ?? ?? ?? 4C 8D 2D ?? ?? ?? ?? 31 D2 4C 89 E6 49 8B 7D 00 48 8B 07 FF 10 \
    A9 30 40 00 00";
    pub(crate) const FLUID_END_TOUCH_SIG: &str =
        "55 48 85 F6 48 89 E5 41 56 49 89 D6 41 55 41 54 53 8B 87 F0 01 00 00 \
    48 89 FB 8D 50 01 89 97 F0 01 00 00 74 ?? 4D 85 F6 74 ?? 48 8B 06 49 89 F4 48 89 F7 \
    FF 90 90 00 00 00 48 85 C0 49 89 C5 74 ?? 48 89 C6 48 89 DF E8 ?? ?? ?? ?? 0F 2F 05 \
    ?? ?? ?? ?? 73 ?? 49 8B 45 00 4C 89 F7 41 81 A5 B8 01 00 00 FF FF F7 FF 4C 8B A0 F0 \
    02 00 00 49 8B 06 FF 50 38 48 63 D0 BE 01 00 00 00 4C 89 EF 41 FF D4 8B 83 F0 01 00 00";
    pub(crate) const BUBBLE_TRAIL_SIG: &str =
        "55 48 89 E5 41 57 41 56 41 55 41 89 D5 41 54 49 89 F4 53 48 89 FB 48 83 EC 68 \
    F3 0F 10 47 08 F3 0F 10 0D ?? ?? ?? ?? F3 0F 58 C8 E8 ?? ?? ?? ?? F3 0F 10 0D ?? ?? ?? ?? \
    4C 89 E7 0F 28 D0 F3 41 0F 10 44 24 08 F3 0F 5C 53 08";
    pub(crate) const DISPATCH_EFFECT_SIG: &str =
        "55 48 89 E5 41 56 41 55 4C 8D 35 ?? ?? ?? ?? 49 89 FD 41 54 4C 8D 65 A0 53 4C 89 E7 \
    48 89 F3 48 83 EC 40 E8 ?? ?? ?? ?? 48 89 DE 4C 89 E7";
    pub(crate) const TE_CREATE_SIG: &str =
        "55 48 89 E5 41 57 41 56 4C 8D 35 ?? ?? ?? ?? 41 55 41 54 4C 8D 25 ?? ?? ?? ?? 53 \
    48 89 FB 48 83 EC 28 48 8D 05 ?? ?? ?? ?? 44 8B 05 ?? ?? ?? ?? 4C 8B 28";
    pub(crate) const PHYSENV_SIG: &str =
        "48 8B 3D ?? ?? ?? ?? 48 8D 95 ?? ?? ?? ?? 4C 89 E6 48 8B 07 FF 50 60";
    pub(crate) const ENTITY_ARRAY_SIG: &str =
        "48 8D 15 ?? ?? ?? ?? 8B 00 48 8B 12 83 F8 FF 74 ?? 89 C1 C1 E8 0E 81 E1 FF 3F 00 00 \
    48 C1 E1 05";
    pub(crate) const EXPLOSION_TARGETS: [&[(&str, usize)]; 3] = [
        &[(
            "55 48 89 E5 41 56 41 55 41 54 53 48 89 FB 48 81 EC 90 00 00 00 80 BF D3 02 00 00 00 \
         0F 84 ?? ?? ?? ?? 48 8B 07 66 0F EF C0 48 C7 45 80 00 00 00 00",
            14,
        )],
        &[(
            "55 48 89 E5 41 54 53 48 89 FB 48 81 EC A0 00 00 00 80 BF D3 02 00 00 01 \
         0F 86 ?? ?? ?? ?? 48 8B 07 66 0F EF C0 48 C7 45 90 00 00 00 00",
            17,
        )],
        &[(
            "55 48 89 E5 41 57 41 56 41 55 41 54 53 48 89 FB 48 81 EC A8 00 00 00 48 8B 3D \
         ?? ?? ?? ?? 48 8B 47 78 48 85 C0 74 ?? 80 38 00 0F 85 ?? ?? ?? ??",
            16,
        )],
    ];
}

#[cfg(feature = "server")]
/// Bytes overwritten at the function entry (14-byte absolute jmp, and on Windows 1 nop
/// to end on an instruction boundary; the Linux prologue ends on one at 14).
#[cfg(any(windows, test))]
pub(crate) const STOLEN: usize = 15;
#[cfg(feature = "server")]
#[cfg(not(any(windows, test)))]
pub(crate) const STOLEN: usize = 14;
#[cfg(feature = "server")]
pub(crate) const TRAMPOLINE_SIZE: usize = STOLEN + 14;
#[cfg(feature = "server")]
#[cfg(any(windows, test))]
pub(crate) const OFF_PEV: usize = 0x30;
#[cfg(feature = "server")]
#[cfg(any(windows, test))]
pub(crate) const OFF_EDICT_INDEX: usize = 6;

/// Entity index of a `CBaseEntity`: Windows reads the edict (`pev`, then the edict's
/// index); Linux reads the entity's own `CBaseHandle` (`m_RefEHandle`, at +8: the vtable
/// is at +0), whose low 14 bits are the index into the entity list.
#[cfg(feature = "server")]
#[cfg(any(windows, test))]
pub(crate) unsafe fn entity_index(this: *const u8) -> Option<i32> {
    unsafe {
        let pev = *this.add(OFF_PEV).cast::<*const u8>();
        if pev.is_null() {
            None
        } else {
            Some(i32::from(*pev.add(OFF_EDICT_INDEX).cast::<i16>()))
        }
    }
}
#[cfg(feature = "server")]
#[cfg(not(any(windows, test)))]
pub(crate) unsafe fn entity_index(this: *const u8) -> Option<i32> {
    unsafe {
        let handle = this
            .add(ENTITY_HANDLE_OFFSET)
            .cast::<u32>()
            .read_unaligned();
        (handle != u32::MAX).then_some((handle & 0x3fff) as i32)
    }
}
#[cfg(feature = "server")]
#[cfg(not(any(windows, test)))]
pub(crate) const ENTITY_HANDLE_OFFSET: usize = 8;

#[cfg(feature = "server")]
/// Signature of the function. `??` bytes are the vtable displacement of the
/// `NetworkStateChanged` call and the offset of `m_nWaterLevel`, which differs between
/// game builds (0x2ab in the default build, 0x2b3 on the dev branch): the offset is read
/// from the match (see `learn_water_level_layout`), which also checks that the four
/// places that name it agree.
#[cfg(any(windows, test))]
pub(crate) const SIG: &str = "48 89 5C 24 08 48 89 6C 24 10 48 89 74 24 18 57 \
                       48 83 EC 20 8B FA 0F B6 A9 ?? ?? 00 00 48 8B F1 \
                       38 91 ?? ?? 00 00 74 17 48 8B 01 48 8D 91 ?? ?? \
                       00 00 FF 90 ?? ?? 00 00 40 88 BE ?? ?? 00 00";
/// Where `m_nWaterLevel`'s four displacements sit in the match of `SIG`.
#[cfg(all(feature = "server", any(windows, test)))]
pub(crate) const WATER_LEVEL_DISP_AT: [usize; 4] = [25, 34, 46, 59];
/// Linux `CBaseEntity::SetWaterLevel`: `m_nWaterLevel` is at +0x2d3 (also a layout check).
#[cfg(feature = "server")]
#[cfg(not(any(windows, test)))]
pub(crate) const SIG: &str = linux_sigs::SIG;

#[cfg(feature = "server")]
pub(crate) static TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);
#[cfg(feature = "server")]
pub(crate) static HITS: AtomicU64 = AtomicU64::new(0);

pub(crate) fn parse_sig(sig: &str) -> Vec<Option<u8>> {
    sig.split_whitespace()
        .map(|b| {
            if b == "??" {
                None
            } else {
                Some(u8::from_str_radix(b, 16).expect("bad signature byte"))
            }
        })
        .collect()
}

/// Offsets of every match of `pat` in `hay`.
pub(crate) fn find_all(hay: &[u8], pat: &[Option<u8>]) -> Vec<usize> {
    let mut out = Vec::new();
    if pat.is_empty() || hay.len() < pat.len() {
        return out;
    }
    let first = pat[0];
    'outer: for i in 0..=(hay.len() - pat.len()) {
        if let Some(b) = first {
            if hay[i] != b {
                continue;
            }
        }
        for (j, p) in pat.iter().enumerate().skip(1) {
            if let Some(b) = p {
                if hay[i + j] != *b {
                    continue 'outer;
                }
            }
        }
        out.push(i);
    }
    out
}

#[cfg(feature = "server")]
/// `jmp [rip+0]; dq hook; nop` - 15 bytes written over the function entry.
pub(crate) fn build_detour(hook: u64) -> [u8; STOLEN] {
    let mut b = [0x90u8; STOLEN];
    b[0] = 0xFF;
    b[1] = 0x25;
    b[2..6].copy_from_slice(&0u32.to_le_bytes());
    b[6..14].copy_from_slice(&hook.to_le_bytes());
    b
}

#[cfg(feature = "server")]
/// The stolen instructions followed by `jmp [rip+0]; dq resume`.
pub(crate) fn build_trampoline(stolen: &[u8], resume: u64) -> [u8; TRAMPOLINE_SIZE] {
    assert_eq!(stolen.len(), STOLEN);
    let mut b = [0u8; TRAMPOLINE_SIZE];
    b[..STOLEN].copy_from_slice(stolen);
    b[STOLEN] = 0xFF;
    b[STOLEN + 1] = 0x25;
    b[STOLEN + 2..STOLEN + 6].copy_from_slice(&0u32.to_le_bytes());
    b[STOLEN + 6..STOLEN + 14].copy_from_slice(&resume.to_le_bytes());
    b
}

engine_abi_item! {
#[cfg(feature = "server")]
/// The replacement for `CBaseEntity::SetWaterLevel`, in the engine's calling
/// convention (see `engine_fn!`).
pub(crate) unsafe fn hook_set_water_level(this: *mut c_void, level: c_int) {
    unsafe {
        let original: engine_fn!((*mut c_void, c_int)) =
            std::mem::transmute(TRAMPOLINE.load(Ordering::Relaxed));
        let mut level = level;
        if !this.is_null() {
            if let Some(idx) = entity_index(this as *const u8) {
                // Never block inside an engine callback. No overrides (the usual
                // case): nothing to look up, so no lock either.
                if OVERRIDE_COUNT.load(Ordering::Relaxed) == 0 {
                    original(this, level);
                    return;
                }
                if let Ok(map) = OVERRIDES.try_lock() {
                    if let Some(&o) = map.get(&idx) {
                        level = c_int::from(o);
                        bump(&HITS);
                    }
                }
            }
        }
        original(this, level);
    }
}
}

/// The handful of OS services needed to patch code.
pub(crate) struct Platform {
    #[cfg(feature = "server")]
    pub alloc_exec: unsafe fn(usize) -> *mut u8,
    /// Makes the range writable; returns the previous protection.
    pub make_writable: unsafe fn(*mut u8, usize) -> Option<u32>,
    pub restore_protect: unsafe fn(*mut u8, usize, u32),
    #[cfg(feature = "server")]
    pub flush: unsafe fn(*mut u8, usize),
}

#[cfg(feature = "server")]
/// `CBaseEntity::UpdateWaterState` - used only to locate the engine's
/// `enginetrace` pointer: the `mov rcx,[rip+disp32]` inside it (disp32 at
/// byte 91) loads the `IEngineTrace` object whose vtable slot 0 is
/// `GetPointContents(const Vector&, ...)`; the result is tested against
/// `MASK_WATER` (0x4030) to set water level 1, 2 and 3.
#[cfg(any(windows, test))]
pub(crate) const UPDATE_WATER_STATE_SIG: &str = "\
    48 8B C4 53 48 83 EC 50 48 8B D9 0F 29 70 E8 48 81 C1 ?? ?? 00 00 48 C7 40 CC 00 00 00 00 \
    4C 8D 40 C8 C7 40 C8 00 00 00 00 48 8D 50 D8 C7 40 D8 00 00 00 3F 0F 57 F6 \
    48 C7 40 DC 00 00 00 3F E8 ?? ?? ?? ?? 33 D2 48 8B CB E8 ?? ?? ?? ?? 33 D2 48 8B CB \
    E8 ?? ?? ?? ?? 48 8B 0D ?? ?? ?? ?? 48 8D 54 24 20 45 33 C0 48 8B 01 FF 10 A9 30 40 00 00";
/// Linux `CBaseEntity::UpdateWaterState`: the `enginetrace` variable is reached with
/// `lea r13,[rip+disp32]` (disp32 at byte 95), then `mov rdi,[r13]; mov rax,[rdi]; call [rax]`.
#[cfg(feature = "server")]
#[cfg(not(any(windows, test)))]
pub(crate) const UPDATE_WATER_STATE_SIG: &str = linux_sigs::UPDATE_WATER_STATE_SIG;
#[cfg(feature = "server")]
#[cfg(any(windows, test))]
pub(crate) const TRACE_GLOBAL_DISP_OFFSET: usize = 91;
#[cfg(feature = "server")]
#[cfg(not(any(windows, test)))]
pub(crate) const TRACE_GLOBAL_DISP_OFFSET: usize = 95;

#[cfg(feature = "client")]
#[cfg(any(windows, test))]
/// client.dll: `C_BaseEntity::UpdateWaterState` loads the client's `enginetrace`
/// the same way (`mov rcx,[rip+disp32]`, `GetPointContents`, test `MASK_WATER`). The
/// pattern is only the load and the test, so it does not depend on the function's
/// frame; disp32 is at byte 3 of the match.
pub(crate) const CLIENT_TRACE_SIG: &str =
    "48 8B 0D ?? ?? ?? ?? 48 8D 54 24 ?? 45 33 C0 48 8B 01 FF 10 A9 30 40 00 00";
#[cfg(feature = "client")]
#[cfg(not(any(windows, test)))]
/// client.so: the same function loads it with `lea r13,[rip+disp32]` (disp32 at byte 3),
/// then `mov rdi,[r13]; mov rax,[rdi]; call [rax]` (`GetPointContents`) and tests `MASK_WATER`.
pub(crate) const CLIENT_TRACE_SIG: &str = linux_sigs::CLIENT_TRACE_SIG;

#[cfg(feature = "client")]
/// The `enginetrace` pointer variable loaded by the instruction at `at`.
pub(crate) const unsafe fn trace_global_from_load(at: *const u8) -> *const *mut c_void {
    unsafe {
        let disp = at.add(3).cast::<i32>().read_unaligned() as isize;
        at.offset(7 + disp).cast::<*mut c_void>()
    }
}

#[cfg(feature = "client")]
/// `IClientEntityList` (client realm). Its vtable slot 3 is GetClientEntity(int).
pub(crate) static CLIENT_ENTITY_LIST: AtomicUsize = AtomicUsize::new(0);
#[cfg(feature = "client")]
pub(crate) const CLIENT_GET_ENTITY_SLOT: usize = 3;

#[cfg(feature = "server")]
/// Address of the `enginetrace` pointer variable referenced by
/// `UpdateWaterState` (`func` = start of that function).
pub(crate) const unsafe fn trace_global_from(func: *const u8) -> *const *mut c_void {
    unsafe {
        let disp = func
            .add(TRACE_GLOBAL_DISP_OFFSET)
            .cast::<i32>()
            .read_unaligned() as isize;
        func.offset(TRACE_GLOBAL_DISP_OFFSET as isize + 4 + disp)
            .cast::<*mut c_void>()
    }
}

pub(crate) static ORIGINAL_GPC: AtomicUsize = AtomicUsize::new(0);

engine_abi_item! {
/// Replacement for `IEngineTrace::GetPointContents` (vtable slot 0). All
/// four register arguments are forwarded untouched, so it does not matter
/// whether the engine's signature has a mask parameter or not. A point inside a volume
/// also reports its contents; so does the end point of the last small-hull trace, when
/// the bullet's own line reaches the surface it stopped at inside a volume.
pub(crate) unsafe fn hook_get_point_contents(
    this: *mut c_void,
    pos: *const f32,
    a3: usize,
    a4: usize,
) -> c_int {
    unsafe {
        let original: engine_fn!((*mut c_void, *const f32, usize, usize) -> c_int) =
            std::mem::transmute(ORIGINAL_GPC.load(Ordering::Relaxed));
        let mut contents = original(this, pos, a3, a4);
        if !pos.is_null() {
            // Never block inside an engine callback.
            let p = [pos.read(), pos.add(1).read(), pos.add(2).read()];
            let mut inside =
                snapshot_if(|v| v.may_contain(p)).map_or(0, |vols| vols.contents_at(p));
            if inside == 0 {
                // The end point of a small-hull trace (a shotgun pellet): the engine asks
                // whether it is in water to decide on a splash. Answer for where the
                // bullet's own line reaches the surface instead of the hull's centre.
                if let Some((end, probe)) = LAST_HULL.with(Cell::get) {
                    // The very floats the trace returned, so an exact comparison.
                    #[allow(clippy::float_cmp)]
                    let same = end == p;
                    if same {
                        if let Some(vols) = snapshot_if(|v| v.may_contain(probe)) {
                            inside = vols.contents_at(probe);
                        }
                        if DEBUG.load(Ordering::Relaxed) {
                            debug_log(format!(
                                "hull pellet end ({:.4}, {:.4}, {:.4}): its line reaches the surface at ({:.4}, {:.4}, {:.4}) -> {}",
                                p[0], p[1], p[2], probe[0], probe[1], probe[2],
                                if inside != 0 { "water" } else { "not water" }
                            ));
                        }
                    }
                }
            }
            if inside != 0 {
                contents |= inside;
                bump(&VOLUME_HITS);
            }
            if inside == 0 && DEBUG.load(Ordering::Relaxed) {
                // A point just outside every volume: a bullet's impact point that lands
                // here is not "in the water", so it gets no splash.
                if let Some(vols) = snapshot_if(|v| v.near(p, 2.0)) {
                    if vols.contents_at(p) == 0 {
                        debug_log(format!(
                            "point ({:.4}, {:.4}, {:.4}) is just outside a volume: not water",
                            p[0], p[1], p[2]
                        ));
                    }
                }
            }
        }
        contents
    }
}
}

/// Swaps vtable slot 0 of the `IEngineTrace` object. No machine code is
/// patched. Returns (slot address, original function pointer).
pub(crate) unsafe fn install_contents_hook(
    trace: *mut c_void,
    p: &Platform,
) -> Result<(usize, usize), &'static str> {
    unsafe {
        if trace.is_null() {
            return Err("enginetrace pointer is null");
        }
        let vtable = (trace as *const *mut usize).read();
        if vtable.is_null() {
            return Err("enginetrace has no vtable");
        }
        let original = vtable.read();
        let hook = hook_get_point_contents as *const () as usize;
        if original == 0 || original == hook {
            return Err("GetPointContents slot is empty or already hooked");
        }
        ORIGINAL_GPC.store(original, Ordering::SeqCst);
        let Some(old) = (p.make_writable)(vtable.cast::<u8>(), 8) else {
            return Err("could not make the vtable writable");
        };
        vtable.write(hook);
        (p.restore_protect)(vtable.cast::<u8>(), 8, old);
        Ok((vtable as usize, original))
    }
}

#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
pub(crate) unsafe fn uninstall_contents_hook(slot: usize, original: usize, p: &Platform) {
    unsafe {
        let slot = slot as *mut usize;
        if slot.read() != hook_get_point_contents as *const () as usize {
            return; // somebody else replaced it after us; leave it alone
        }
        if let Some(old) = (p.make_writable)(slot.cast::<u8>(), 8) {
            slot.write(original);
            (p.restore_protect)(slot.cast::<u8>(), 8, old);
        }
    }
}

// ---- Water volumes vs. traces: IEngineTrace::TraceRay (vtable slot 4) --
//
// Verified against 130 TraceRay call sites in the 64-bit server.dll:
//   TraceRay(this, const Ray_t*, unsigned mask, ITraceFilter*, trace_t*)
//   Ray_t:   m_Start +0x00, m_Delta +0x10, m_StartOffset +0x20,
//            m_Extents +0x30 (3 floats each), m_IsRay +0x40, m_IsSwept +0x41
//   trace_t: startpos +0x00, endpos +0x0c, plane{normal +0x18, dist +0x24,
//            type +0x28, signbits +0x29}, fraction +0x2c, contents +0x30,
//            dispFlags +0x34 (u16), allsolid +0x36, startsolid +0x37,
//            fractionleftsolid +0x38, surface{name +0x40, props +0x48,
//            flags +0x4a}, hitgroup +0x50, physicsbone +0x54, m_pEnt +0x58,
//            hitbox +0x60

pub(crate) const TRACE_RAY_SLOT: usize = 4;
pub(crate) static ORIGINAL_TRACE_RAY: AtomicUsize = AtomicUsize::new(0);
pub(crate) static TRACE_HITS: AtomicU64 = AtomicU64::new(0);

/// While set, the hooks log what they answered near a volume (`waterleveler_debug 1`).
pub(crate) static DEBUG: AtomicBool = AtomicBool::new(false);
static DEBUG_LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Adds a line to the debug log (kept to the latest 512; never blocks).
fn debug_log(line: String) {
    if let Ok(mut log) = DEBUG_LOG.try_lock() {
        if log.len() < 512 {
            log.push(line);
        }
    }
}

/// The debug log so far, one line per entry; empties it.
pub(crate) fn take_debug_log() -> String {
    let mut log = DEBUG_LOG
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let text = log.join("\n");
    log.clear();
    text
}

/// Offset of `Ray_t::m_IsSwept`. The older 64-bit builds have it at +0x41
/// (`m_IsRay` +0x40); the x86-64 branch inserted an 8-byte member (the world
/// axis transform pointer) at +0x40, so the flags sit at +0x48/+0x49.
pub(crate) const RAY_SWEPT_OFFSET_OLD: usize = 0x41;
pub(crate) static RAY_SWEPT_OFFSET: AtomicUsize = AtomicUsize::new(RAY_SWEPT_OFFSET_OLD);

/// The engine helper that builds a `Ray_t` on its stack from two points
/// (`UTIL_TraceLine`-style): its `mov byte [rax+disp],1` stores `m_IsSwept`.
/// The frame is `mov rax,rsp; push rbx; sub rsp,0x90` with the `Ray_t` at
/// rsp+0x40, so offset = disp + 0x98 - 0x40. Byte 53 is disp.
pub(crate) const RAY_HELPER_SIG: &str =
    "48 8B C4 53 48 81 EC 90 00 00 00 F3 0F 10 42 08 F3 0F 10 19 \
    F3 0F 10 0A F3 0F 10 61 04 F3 0F 5C CB F3 0F 10 52 04 F3 0F 10 69 08 F3 0F 5C D4 F3 0F 5C C5 \
    C6 40 ?? 01";
pub(crate) const RAY_HELPER_DISP_BYTE: usize = 53;

pub(crate) const unsafe fn ray_swept_offset_from(func: *const u8) -> usize {
    unsafe {
        let disp = func.add(RAY_HELPER_DISP_BYTE).cast::<i8>().read() as isize;
        (disp + 0x98 - 0x40) as usize
    }
}

/// How far inside a volume, along the axis of the face it entered through, a rewritten
/// trace ends. More than the engine's 1/32 trace epsilon and its 0.1 downward probe
/// can cancel out.
pub(crate) const ENTRY_DEPTH: f32 = 0.125;

/// The surface name a rewritten trace reports, as a water brush's is.
static WATER_SURFACE_NAME: &[u8] = b"water\0";

/// Where the segment `s + t*d` (t in 0..1) first enters the box `[lo, hi]`
/// from outside: `(t, axis, entered_through_min_face)`. `None` if it never
/// does, or if the start is already inside or on the boundary.
pub(crate) fn segment_enters_box(
    s: [f32; 3],
    d: [f32; 3],
    lo: [f32; 3],
    hi: [f32; 3],
) -> Option<(f32, usize, bool)> {
    let mut tmin = f32::NEG_INFINITY;
    let mut tmax = f32::INFINITY;
    let mut axis = 0usize;
    let mut min_face = true;
    for i in 0..3 {
        if d[i].abs() < 1e-9 {
            if s[i] < lo[i] || s[i] > hi[i] {
                return None;
            }
            continue;
        }
        let t1 = (lo[i] - s[i]) / d[i];
        let t2 = (hi[i] - s[i]) / d[i];
        let (near, far, through_min) = if t1 < t2 {
            (t1, t2, true)
        } else {
            (t2, t1, false)
        };
        if near > tmin {
            tmin = near;
            axis = i;
            min_face = through_min;
        }
        tmax = tmax.min(far);
        if tmin > tmax {
            return None;
        }
    }
    if tmin.is_finite() && tmin > 0.0 && tmin < 1.0 {
        Some((tmin, axis, min_face))
    } else {
        None
    }
}

/// Where the segment `s + t*d` (t in 0..=1) leaves the box `[lo, hi]` that it
/// starts in: the exit fraction, or 1.0 if it stays inside.
pub(crate) fn segment_exit_fraction(s: [f32; 3], d: [f32; 3], lo: [f32; 3], hi: [f32; 3]) -> f32 {
    let mut exit = 1.0f32;
    for i in 0..3 {
        if d[i].abs() < 1e-9 {
            continue;
        }
        let far = if d[i] > 0.0 { hi[i] } else { lo[i] };
        exit = exit.min(((far - s[i]) / d[i]).max(0.0));
    }
    exit
}

/// If the ray enters a water volume before `trace` currently ends, rewrites
/// `trace` as a water-surface hit. Returns true if it did.
#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
pub(crate) unsafe fn apply_water_to_trace(
    vols: &[Volume],
    ray: *const u8,
    trace: *mut u8,
    world: usize,
) -> bool {
    unsafe { apply_water_to_trace_masked(vols, ray, trace, world, u32::MAX) }
}

/// Same, only for the volumes whose contents `mask` asks for.
pub(crate) unsafe fn apply_water_to_trace_masked(
    vols: &[Volume],
    ray: *const u8,
    trace: *mut u8,
    world: usize,
    mask: u32,
) -> bool {
    unsafe {
        apply_water_to_trace_at(
            vols,
            ray,
            trace,
            world,
            RAY_SWEPT_OFFSET.load(Ordering::Relaxed),
            mask,
        )
    }
}

/// The fields of a `Ray_t` the water logic needs.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RayFields {
    pub(crate) start: [f32; 3],
    pub(crate) delta: [f32; 3],
    pub(crate) offset: [f32; 3],
    pub(crate) extents: [f32; 3],
}

/// What a ray does to a trace result, decided by `plan_trace` and written by
/// `apply_water_to_trace_at`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum TraceOutcome {
    /// The ray starts and ends inside a volume: allsolid, fraction 0.
    Inside { origin: [f32; 3], contents: c_int },
    /// The ray starts inside a volume and leaves it: startsolid, plus the fraction at
    /// which it left (when that is earlier than what the trace already has).
    Leaves { left_at: Option<f32> },
    /// The ray enters a volume from outside. `fraction` and `end` are the entry point, or
    /// for a point ray (a bullet) `ENTRY_DEPTH` further in, so the engine's probe 0.1 below
    /// the end of the trace is inside the volume; `plane_dist` is the face's.
    Enters {
        origin: [f32; 3],
        end: [f32; 3],
        fraction: f32,
        axis: usize,
        min_face: bool,
        plane_dist: f32,
        contents: c_int,
    },
}

/// The water logic, on plain values: what `ray` does to a trace whose fraction is
/// `current`, counting only the volumes whose contents `mask` asks for. `None` leaves
/// the trace alone.
pub(crate) fn plan_trace(
    vols: &[Volume],
    ray: &RayFields,
    current: f32,
    mask: u32,
) -> Option<TraceOutcome> {
    let RayFields {
        start: s,
        delta: d,
        offset: off,
        extents: ext,
    } = *ray;
    let origin = [s[0] + off[0], s[1] + off[1], s[2] + off[2]];

    // Rays that START inside a volume. A real water brush reports these as
    // startsolid, and when the whole ray stays inside it as allsolid with
    // fraction 0 and the end point at the start. The engine's water-height
    // probe (a downward MASK_WATER ray from above the entity)
    // depends on that: without it a probe starting inside a volume looks
    // like "no water here" and the computed surface height collapses to
    // the entity's own height.
    let mut started_inside = false;
    let mut left_at = 1.0f32;
    for v in vols.iter().filter(|v| v.contents as u32 & mask != 0) {
        let lo = [v.min[0] - ext[0], v.min[1] - ext[1], v.min[2] - ext[2]];
        let hi = [v.max[0] + ext[0], v.max[1] + ext[1], v.max[2] + ext[2]];
        let inside = |p: [f32; 3]| (0..3).all(|i| p[i] >= lo[i] && p[i] <= hi[i]);
        if !inside(s) {
            continue;
        }
        started_inside = true;
        left_at = left_at.min(segment_exit_fraction(s, d, lo, hi));
        let e = [s[0] + d[0], s[1] + d[1], s[2] + d[2]];
        if inside(e) {
            return Some(TraceOutcome::Inside {
                origin,
                contents: v.contents,
            });
        }
    }
    if started_inside {
        // Where it left: the client's water explosion measures the depth from that.
        return Some(TraceOutcome::Leaves {
            left_at: (left_at < current).then_some(left_at),
        });
    }

    let mut best: Option<(f32, usize, bool, usize)> = None;
    for (i, v) in vols
        .iter()
        .enumerate()
        .filter(|(_, v)| v.contents as u32 & mask != 0)
    {
        let lo = [v.min[0] - ext[0], v.min[1] - ext[1], v.min[2] - ext[2]];
        let hi = [v.max[0] + ext[0], v.max[1] + ext[1], v.max[2] + ext[2]];
        if let Some((t, axis, min_face)) = segment_enters_box(s, d, lo, hi) {
            if t < current && best.map_or(true, |b| t < b.0) {
                best = Some((t, axis, min_face, i));
            }
        }
    }
    let (t_entry, axis, min_face, vi) = best?;
    let v = &vols[vi];
    // The engine decides whether a bullet hit water by testing the point 0.1 straight
    // below the end of this trace (`HandleShotImpactingWater`). A real water brush only
    // has a top face, so that point is always under the surface. A volume has side faces
    // too: a trace ending exactly on one has that point on the face's edge, or outside
    // it, and the shot gets no splash. End the trace slightly inside the volume instead.
    let lo = [v.min[0] - ext[0], v.min[1] - ext[1], v.min[2] - ext[2]];
    let hi = [v.max[0] + ext[0], v.max[1] + ext[1], v.max[2] + ext[2]];
    let t_exit = segment_exit_fraction(s, d, lo, hi);
    // Only for point rays (bullets). A hull (player movement, water-level probes) rests
    // against the face as before.
    let point_ray = ext.iter().all(|&e| e == 0.0);
    let push = if point_ray {
        ENTRY_DEPTH / d[axis].abs().max(1e-9)
    } else {
        0.0
    };
    let t = (t_entry + push)
        .min((t_entry + t_exit) * 0.5)
        .min((current - t_entry).mul_add(0.5, t_entry))
        .max(t_entry);
    let mut end = origin;
    for i in 0..3 {
        end[i] = d[i].mul_add(t, origin[i]);
    }
    Some(TraceOutcome::Enters {
        origin,
        end,
        fraction: t,
        axis,
        min_face,
        plane_dist: if min_face { -v.min[axis] } else { v.max[axis] },
        contents: v.contents,
    })
}

/// Same, with the offset of `Ray_t::m_IsSwept` given explicitly. Only the reads of the
/// ray and the writes of the trace are here; `plan_trace` decides what to write.
pub(crate) unsafe fn apply_water_to_trace_at(
    vols: &[Volume],
    ray: *const u8,
    trace: *mut u8,
    world: usize,
    swept_offset: usize,
    mask: u32,
) -> bool {
    unsafe {
        // A trace that reports a hit must name the entity it hit: callers
        // (the SLAM's beam, impact effects) call a virtual on `m_pEnt`. A real
        // water brush belongs to the world, so the world entity is used. If it
        // is unknown and the engine found no entity, leave the trace alone.
        let ent_slot = trace.add(0x58).cast::<usize>();
        let ent = if world != 0 {
            world
        } else {
            ent_slot.read_unaligned()
        };
        if ent == 0 {
            return false;
        }
        if ray.add(swept_offset).read() == 0 {
            return false; // not swept: nothing travels
        }
        let rf = |o: usize| ray.add(o).cast::<f32>().read_unaligned();
        let fields = RayFields {
            start: [rf(0x00), rf(0x04), rf(0x08)],
            delta: [rf(0x10), rf(0x14), rf(0x18)],
            offset: [rf(0x20), rf(0x24), rf(0x28)],
            extents: [rf(0x30), rf(0x34), rf(0x38)],
        };
        let current = trace.add(0x2c).cast::<f32>().read_unaligned();
        let Some(outcome) = plan_trace(vols, &fields, current, mask) else {
            return false;
        };

        let wf = |o: usize, val: f32| trace.add(o).cast::<f32>().write_unaligned(val);
        match outcome {
            TraceOutcome::Inside { origin, contents } => {
                for (i, &component) in origin.iter().enumerate() {
                    wf(i * 4, component);
                    wf(0x0c + i * 4, component);
                    wf(0x18 + i * 4, 0.0);
                }
                wf(0x24, 0.0);
                trace.add(0x28).write(0);
                trace.add(0x29).write(0);
                wf(0x2c, 0.0); // fraction
                trace.add(0x30).cast::<i32>().write_unaligned(contents);
                trace.add(0x34).cast::<u16>().write_unaligned(0);
                trace.add(0x36).write(1); // allsolid
                trace.add(0x37).write(1); // startsolid
                wf(0x38, 0.0);
                trace
                    .add(0x40)
                    .cast::<usize>()
                    .write_unaligned(WATER_SURFACE_NAME.as_ptr() as usize);
                ent_slot.write_unaligned(ent);
            }
            TraceOutcome::Leaves { left_at } => {
                trace.add(0x37).write(1);
                if let Some(fraction) = left_at {
                    wf(0x38, fraction);
                }
            }
            TraceOutcome::Enters {
                origin,
                end,
                fraction,
                axis,
                min_face,
                plane_dist,
                contents,
            } => {
                for i in 0..3 {
                    wf(i * 4, origin[i]); // startpos
                    wf(0x0c + i * 4, end[i]); // endpos
                    wf(0x18 + i * 4, 0.0); // plane.normal
                }
                wf(0x18 + axis * 4, if min_face { -1.0 } else { 1.0 });
                wf(0x24, plane_dist);
                trace.add(0x28).write(axis as u8); // plane.type: axial
                trace
                    .add(0x29)
                    .write(if min_face { 1u8 << axis } else { 0 }); // signbits
                wf(0x2c, fraction);
                trace.add(0x30).cast::<i32>().write_unaligned(contents);
                trace.add(0x34).cast::<u16>().write_unaligned(0); // dispFlags
                trace.add(0x36).write(0); // allsolid
                trace.add(0x37).write(0); // startsolid
                wf(0x38, 0.0); // fractionleftsolid
                trace
                    .add(0x40)
                    .cast::<usize>()
                    .write_unaligned(WATER_SURFACE_NAME.as_ptr() as usize);
                trace.add(0x48).cast::<i16>().write_unaligned(0); // surface.surfaceProps
                trace.add(0x4a).cast::<u16>().write_unaligned(0); // surface.flags
                trace.add(0x50).cast::<i32>().write_unaligned(0); // hitgroup
                trace.add(0x54).cast::<i16>().write_unaligned(0); // physicsbone
                trace.add(0x60).cast::<i32>().write_unaligned(0); // hitbox
                ent_slot.write_unaligned(ent); // m_pEnt
            }
        }
        true
    }
}

/// True if the ray certainly touches no volume: the box around its segment (grown by
/// the ray's extents) is separated from the box around every volume. Anything else,
/// NaN included, is "maybe". Reads the same ray fields as `apply_water_to_trace_at`.
pub(crate) unsafe fn ray_misses_bounds(
    bounds: Option<([f32; 3], [f32; 3])>,
    ray: *const u8,
) -> bool {
    unsafe {
        let rf = |o: usize| ray.add(o).cast::<f32>().read_unaligned();
        segment_misses_bounds(
            bounds,
            [rf(0x00), rf(0x04), rf(0x08)],
            [rf(0x10), rf(0x14), rf(0x18)],
            [rf(0x30), rf(0x34), rf(0x38)],
        )
    }
}

/// True if the segment from `start` along `delta`, grown by `extents`, certainly stays
/// outside `bounds` (no bounds: no volumes). NaN counts as "maybe".
pub(crate) fn segment_misses_bounds(
    bounds: Option<([f32; 3], [f32; 3])>,
    start: [f32; 3],
    delta: [f32; 3],
    extents: [f32; 3],
) -> bool {
    let Some((lo, hi)) = bounds else {
        return true; // no volumes
    };
    (0..3).any(|i| {
        let e = start[i] + delta[i];
        let (smin, smax) = if e < start[i] {
            (e, start[i])
        } else {
            (start[i], e)
        };
        smax < lo[i] - extents[i] || smin > hi[i] + extents[i]
    })
}

engine_abi_item! {
/// Replacement for `IEngineTrace::TraceRay`: runs the engine's trace, then, for a trace
/// with a water bit in its mask, lets `plan_trace` rewrite the result for the volumes the
/// ray touches. A trace without one is only remembered when it is a small hull (a shotgun
/// pellet), see `note_small_hull`.
pub(crate) unsafe fn hook_trace_ray(
    this: *mut c_void,
    ray: *const u8,
    mask: u32,
    filter: *mut c_void,
    trace: *mut u8,
) {
    unsafe {
        let original: engine_fn!((
            *mut c_void,
            *const u8,
            u32,
            *mut c_void,
            *mut u8,
        )) = std::mem::transmute(ORIGINAL_TRACE_RAY.load(Ordering::Relaxed));
        original(this, ray, mask, filter, trace);
        if ray.is_null() || trace.is_null() {
            return;
        }
        if mask & ((CONTENTS_WATER | CONTENTS_SLIME) as u32) == 0 {
            note_small_hull(ray, mask, trace);
            return;
        }
        // Never block inside an engine callback.
        // A ray far from every volume changes nothing (and needs no world entity).
        if let Some(vols) = snapshot_if(|v| !ray_misses_bounds(v.bounds, ray)) {
            let debug = DEBUG.load(Ordering::Relaxed);
            let before = if debug {
                Some(trace_summary(trace))
            } else {
                None
            };
            // Only the volumes whose contents the trace's mask asks for are considered.
            let world = world_entity();
            let hit = apply_water_to_trace_masked(&vols, ray, trace, world, mask);
            if hit {
                bump(&TRACE_HITS);
            }
            // A trace that changed nothing and started far from every volume (the sun's
            // line to the player, say) is not worth a log line.
            let start = [
                ray.cast::<f32>().read_unaligned(),
                ray.add(4).cast::<f32>().read_unaligned(),
                ray.add(8).cast::<f32>().read_unaligned(),
            ];
            let worth_logging = hit || vols.near(start, 2048.0);
            if let (Some((fraction, ent)), true) = (before, worth_logging) {
                let rf = |o: usize| ray.add(o).cast::<f32>().read_unaligned();
                let (after, _) = trace_summary(trace);
                debug_log(format!(
                    "trace mask {mask:#x} from ({:.3}, {:.3}, {:.3}) by ({:.3}, {:.3}, {:.3}) extents ({:.3}, {:.3}, {:.3}): swept {}, engine fraction {fraction:.5} entity {ent:#x}, world {world:#x} -> {} (fraction {after:.5}, startsolid {}, allsolid {})",
                    rf(0x00), rf(0x04), rf(0x08), rf(0x10), rf(0x14), rf(0x18),
                    rf(0x30), rf(0x34), rf(0x38),
                    ray.add(RAY_SWEPT_OFFSET.load(Ordering::Relaxed)).read(),
                    if hit { "rewritten" } else { "left alone" },
                    trace.add(0x37).read(),
                    trace.add(0x36).read(),
                ));
            }
        }
    }
}
}

thread_local! {
    /// The last small-hull trace that hit something near a volume: (its end point, the
    /// point where the bullet's own line reaches the surface the hull stopped at).
    static LAST_HULL: Cell<Option<([f32; 3], [f32; 3])>> = const { Cell::new(None) };
}

/// Where the centre line of a hull trace reaches the surface the hull stopped at. The
/// hull's centre rests `extents . |normal|` away from that surface, so for a bullet
/// that went into a volume through a side face and hit the floor next to it the centre
/// can end outside the volume while the bullet itself is inside.
pub(crate) fn hull_surface_point(
    end: [f32; 3],
    normal: [f32; 3],
    delta: [f32; 3],
    extents: [f32; 3],
) -> Option<[f32; 3]> {
    let len = delta[2]
        .mul_add(delta[2], delta[0].mul_add(delta[0], delta[1] * delta[1]))
        .sqrt();
    if len.is_nan() || len <= 1e-3 {
        return None;
    }
    let dir = [delta[0] / len, delta[1] / len, delta[2] / len];
    let toward = -dir[2].mul_add(normal[2], dir[0].mul_add(normal[0], dir[1] * normal[1]));
    if toward.is_nan() || toward <= 0.01 {
        return None;
    }
    let rest = extents[2].mul_add(
        normal[2].abs(),
        extents[1].mul_add(normal[1].abs(), extents[0] * normal[0].abs()),
    );
    let k = rest / toward;
    Some([
        dir[0].mul_add(k, end[0]),
        dir[1].mul_add(k, end[1]),
        dir[2].mul_add(k, end[2]),
    ])
}

/// A trace without the water bit whose hull is a few units wide (the shotgun's odd
/// pellets use a 3 unit hull) that hit something near a volume: remembers where its
/// line reaches the surface, for `hook_get_point_contents`, and logs it when debugging.
unsafe fn note_small_hull(ray: *const u8, mask: u32, trace: *const u8) {
    unsafe {
        let rf = |o: usize| ray.add(o).cast::<f32>().read_unaligned();
        let ext = [rf(0x30), rf(0x34), rf(0x38)];
        if !ext.iter().all(|&e| (1.0..=5.0).contains(&e)) {
            return;
        }
        let (fraction, ent) = trace_summary(trace);
        if fraction >= 1.0 {
            return;
        }
        if snapshot_if(|v| !ray_misses_bounds(v.bounds, ray)).is_none() {
            return;
        }
        let tf = |o: usize| trace.add(o).cast::<f32>().read_unaligned();
        let end = [tf(0x0c), tf(0x10), tf(0x14)];
        let normal = [tf(0x18), tf(0x1c), tf(0x20)];
        let delta = [rf(0x10), rf(0x14), rf(0x18)];
        if let Some(probe) = hull_surface_point(end, normal, delta, ext) {
            LAST_HULL.with(|c| c.set(Some((end, probe))));
        }
        if DEBUG.load(Ordering::Relaxed) {
            debug_log(format!(
                "hull trace mask {mask:#x} from ({:.3}, {:.3}, {:.3}) by ({:.3}, {:.3}, {:.3}) extents ({:.1}, {:.1}, {:.1}): fraction {fraction:.5} entity {ent:#x}, end ({:.3}, {:.3}, {:.3})",
                rf(0x00), rf(0x04), rf(0x08), delta[0], delta[1], delta[2],
                ext[0], ext[1], ext[2],
                end[0], end[1], end[2],
            ));
        }
    }
}

/// (fraction, hit entity) of a trace result.
const unsafe fn trace_summary(trace: *const u8) -> (f32, usize) {
    unsafe {
        (
            trace.add(0x2c).cast::<f32>().read_unaligned(),
            trace.add(0x58).cast::<usize>().read_unaligned(),
        )
    }
}

/// Swaps vtable slot `slot` of the `IEngineTrace` object for `hook`.
/// Returns (address of the slot, original function pointer).
pub(crate) unsafe fn install_slot_hook(
    trace: *mut c_void,
    slot: usize,
    hook: usize,
    original_store: &AtomicUsize,
    p: &Platform,
) -> Result<(usize, usize), &'static str> {
    unsafe {
        if trace.is_null() {
            return Err("enginetrace pointer is null");
        }
        let vtable = (trace as *const *mut usize).read();
        if vtable.is_null() {
            return Err("enginetrace has no vtable");
        }
        let entry = vtable.add(slot);
        let original = entry.read();
        if original == 0 || original == hook {
            return Err("vtable slot is empty or already hooked");
        }
        original_store.store(original, Ordering::SeqCst);
        let Some(old) = (p.make_writable)(entry.cast::<u8>(), 8) else {
            return Err("could not make the vtable writable");
        };
        entry.write(hook);
        (p.restore_protect)(entry.cast::<u8>(), 8, old);
        Ok((entry as usize, original))
    }
}

pub(crate) unsafe fn uninstall_slot_hook(entry: usize, hook: usize, original: usize, p: &Platform) {
    unsafe {
        let entry = entry as *mut usize;
        if entry.read() != hook {
            return; // somebody else replaced it after us; leave it alone
        }
        if let Some(old) = (p.make_writable)(entry.cast::<u8>(), 8) {
            entry.write(original);
            (p.restore_protect)(entry.cast::<u8>(), 8, old);
        }
    }
}

#[cfg(feature = "server")]
/// Walks the executable sections of a loaded PE image (`base` = module
/// base) and returns the single match of [`SIG`].
pub(crate) unsafe fn scan_image(base: *const u8) -> Result<*mut u8, &'static str> {
    unsafe { scan_sig(base, SIG) }
}

#[cfg(feature = "server")]
/// `83 F9 FF  74 ??  48 8B 35 <entity array>  8B C1  25 FF 3F 00 00
/// C1 E9 0E  48 C1 E0 05  39 4C 30 10`: the engine's "entity from handle"
/// idiom (entries are 32 bytes: pointer at +8, serial at +0x10). It occurs
/// many times, always with the same array global.
#[cfg(any(windows, test))]
pub(crate) const ENTITY_ARRAY_SIG: &str =
    "83 F9 FF 74 ?? 48 8B 35 ?? ?? ?? ?? 8B C1 25 FF 3F 00 00 \
                                    C1 E9 0E 48 C1 E0 05 39 4C 30 10";
#[cfg(feature = "server")]
#[cfg(not(any(windows, test)))]
pub(crate) const ENTITY_ARRAY_SIG: &str = linux_sigs::ENTITY_ARRAY_SIG;
/// Where the rip-relative displacement of the array global sits in the idiom.
#[cfg(feature = "server")]
#[cfg(any(windows, test))]
const ENTITY_ARRAY_DISP_AT: usize = 8;
#[cfg(feature = "server")]
#[cfg(not(any(windows, test)))]
const ENTITY_ARRAY_DISP_AT: usize = 3;
#[cfg(feature = "server")]
pub(crate) static ENTITY_ARRAY_GLOBAL: AtomicUsize = AtomicUsize::new(0);

#[cfg(feature = "server")]
/// Address of the global holding the entity array, if every match of the
/// idiom agrees on it.
pub(crate) unsafe fn entity_array_global(base: *const u8) -> Result<usize, &'static str> {
    unsafe {
        let all = scan_sig_all(base, ENTITY_ARRAY_SIG)?;
        let mut global = 0usize;
        for at in all {
            let disp = (at.add(ENTITY_ARRAY_DISP_AT) as *const i32).read_unaligned() as isize;
            let g = at.offset((ENTITY_ARRAY_DISP_AT + 4) as isize + disp) as usize;
            if global != 0 && global != g {
                return Err("entity array idiom resolves to different globals");
            }
            global = g;
        }
        if global == 0 {
            return Err("entity array idiom not found");
        }
        Ok(global)
    }
}

/// The world entity (entry 0 of the entity array), or 0 if unknown.
#[allow(clippy::needless_return)] // the server branch is the tail only when no client branch exists
pub(crate) unsafe fn world_entity() -> usize {
    unsafe {
        #[cfg(feature = "client")]
        {
            let list = CLIENT_ENTITY_LIST.load(Ordering::Relaxed);
            if list != 0 {
                // Client realm: entity 0 of the client's entity list.
                let vtable = (list as *const *const usize).read();
                let get: engine_fn!((usize, c_int) -> usize) =
                    std::mem::transmute(vtable.add(CLIENT_GET_ENTITY_SLOT).read());
                return get(list, 0);
            }
        }
        #[cfg(feature = "server")]
        {
            let g = ENTITY_ARRAY_GLOBAL.load(Ordering::Relaxed);
            if g == 0 {
                return 0;
            }
            let array = (g as *const usize).read_unaligned();
            if array == 0 {
                return 0;
            }
            return ((array + 8) as *const usize).read_unaligned();
        }
        #[cfg(not(feature = "server"))]
        0
    }
}

/// The executable parts of a loaded module as (offset from `base`, size): the
/// executable sections of a PE image, the executable segments of an ELF one.
#[cfg(all(windows, any(feature = "server", feature = "client")))]
unsafe fn executable_regions(base: *const u8) -> Result<Vec<(usize, usize)>, &'static str> {
    unsafe { pe_regions(base) }
}

/// The tests scan mapped Windows images on a Linux host too.
#[cfg(all(any(feature = "server", feature = "client"), not(windows)))]
unsafe fn executable_regions(base: *const u8) -> Result<Vec<(usize, usize)>, &'static str> {
    unsafe {
        #[cfg(test)]
        if std::slice::from_raw_parts(base, 2) == b"MZ" {
            return pe_regions(base);
        }
        elf_regions(base)
    }
}

#[cfg(all(any(windows, test), any(feature = "server", feature = "client")))]
unsafe fn pe_regions(base: *const u8) -> Result<Vec<(usize, usize)>, &'static str> {
    unsafe {
        let rd32 = |off: usize| base.add(off).cast::<u32>().read_unaligned();
        let rd16 = |off: usize| base.add(off).cast::<u16>().read_unaligned();
        if rd16(0) != 0x5A4D {
            return Err("module has no MZ header");
        }
        let nt = rd32(0x3C) as usize;
        if rd32(nt) != 0x0000_4550 {
            return Err("module has no PE header");
        }
        if rd16(nt + 24) != 0x20B {
            return Err("module is not a 64-bit PE image");
        }
        let nsec = rd16(nt + 6) as usize;
        let optsz = rd16(nt + 20) as usize;
        let sections = nt + 24 + optsz;
        let mut regions = Vec::new();
        for i in 0..nsec {
            let s = sections + i * 40;
            if rd32(s + 36) & 0x2000_0000 != 0 {
                regions.push((rd32(s + 12) as usize, rd32(s + 8) as usize));
            }
        }
        Ok(regions)
    }
}

#[cfg(all(any(feature = "server", feature = "client"), not(windows)))]
unsafe fn elf_regions(base: *const u8) -> Result<Vec<(usize, usize)>, &'static str> {
    unsafe {
        const PT_LOAD: u32 = 1;
        const PF_X: u32 = 1;
        let rd64 = |off: usize| base.add(off).cast::<u64>().read_unaligned();
        let rd32 = |off: usize| base.add(off).cast::<u32>().read_unaligned();
        let rd16 = |off: usize| base.add(off).cast::<u16>().read_unaligned();
        if std::slice::from_raw_parts(base, 4) != b"\x7fELF" {
            return Err("module has no ELF header");
        }
        if *base.add(4) != 2 || rd16(18) != 62 {
            return Err("module is not a 64-bit x86-64 ELF image");
        }
        let phoff = rd64(0x20) as usize;
        let phentsize = rd16(0x36) as usize;
        let phnum = rd16(0x38) as usize;
        let mut regions = Vec::new();
        for i in 0..phnum {
            let ph = phoff + i * phentsize;
            if rd32(ph) == PT_LOAD && rd32(ph + 4) & PF_X != 0 {
                // p_vaddr, p_memsz
                regions.push((rd64(ph + 0x10) as usize, rd64(ph + 0x28) as usize));
            }
        }
        Ok(regions)
    }
}

/// Every match of a signature in the executable parts of the module.
#[cfg(any(feature = "server", feature = "client"))]
pub(crate) unsafe fn scan_sig_all(
    base: *const u8,
    sig: &str,
) -> Result<Vec<*mut u8>, &'static str> {
    unsafe {
        let pat = parse_sig(sig);
        let mut found: Vec<*mut u8> = Vec::new();
        for (va, size) in executable_regions(base)? {
            let region = std::slice::from_raw_parts(base.add(va), size);
            for off in find_all(region, &pat) {
                found.push(base.add(va + off).cast_mut());
            }
        }
        if found.is_empty() {
            return Err("signature not found (different game build?)");
        }
        Ok(found)
    }
}

/// Same, for a signature that must match exactly once.
#[cfg(any(feature = "server", feature = "client"))]
pub(crate) unsafe fn scan_sig(base: *const u8, sig: &str) -> Result<*mut u8, &'static str> {
    unsafe {
        let found = scan_sig_all(base, sig)?;
        if found.len() == 1 {
            Ok(found[0])
        } else {
            Err("signature is ambiguous")
        }
    }
}

#[cfg(feature = "server")]
/// Detours `target`. Returns the original bytes for [`uninstall`].
pub(crate) unsafe fn install(target: *mut u8, p: &Platform) -> Result<[u8; STOLEN], &'static str> {
    unsafe {
        let pat = parse_sig(SIG);
        let mut original = [0u8; STOLEN];
        original.copy_from_slice(std::slice::from_raw_parts(target, STOLEN));
        for (i, want) in pat.iter().take(STOLEN).enumerate() {
            if let Some(w) = want {
                if original[i] != *w {
                    return Err("function prologue does not match");
                }
            }
        }

        let tramp = (p.alloc_exec)(64);
        if tramp.is_null() {
            return Err("could not allocate executable memory");
        }
        let tb = build_trampoline(&original, target as u64 + STOLEN as u64);
        std::ptr::copy_nonoverlapping(tb.as_ptr(), tramp, tb.len());
        (p.flush)(tramp, tb.len());
        TRAMPOLINE.store(tramp as usize, Ordering::SeqCst);

        let Some(old) = (p.make_writable)(target, STOLEN) else {
            return Err("could not make the function writable");
        };
        let detour = build_detour(hook_set_water_level as *const () as usize as u64);
        std::ptr::copy_nonoverlapping(detour.as_ptr(), target, STOLEN);
        (p.restore_protect)(target, STOLEN, old);
        (p.flush)(target, STOLEN);
        Ok(original)
    }
}

#[cfg(feature = "server")]
pub(crate) unsafe fn uninstall(target: *mut u8, original: &[u8; STOLEN], p: &Platform) {
    unsafe {
        if let Some(old) = (p.make_writable)(target, STOLEN) {
            std::ptr::copy_nonoverlapping(original.as_ptr(), target, STOLEN);
            (p.restore_protect)(target, STOLEN, old);
            (p.flush)(target, STOLEN);
        }
    }
}

// ---- Fluid controllers (buoyancy) --------------------------------------
//
// How a map's water brush becomes buoyant, read from the 64-bit server.dll
// (world setup) and vphysics.dll:
//   obj = physenv->CreatePolyObjectStatic(collide, <water material>, ...)   // slot 8
//   physenv->CreateFluidController(obj, &fluidparams_t)                     // slot 11
// CreateFluidController (vphysics) takes the fluid's density from
// the object's *surface-property material* (so the object must use
// "water"), reads the surface plane (WORLD space, normal pointing out of
// the fluid), current velocity, damping (+0x1c), game data (+0x28) and
// contents (+0x34), and makes the object a trigger. Props that overlap it
// then get buoyancy and drag from vphysics. DestroyFluidController is
// slot 12 (SDK order; slots 8 and 11 were both confirmed).

#[cfg(feature = "server")]
/// Finds `mov rcx,[physenv]; lea r8,[..]; mov rdx,rsi; mov r9,[rcx];
/// call [r9+0x58]` in the world-setup code. The rip displacement of the
/// first instruction is the `physenv` global.
#[cfg(any(windows, test))]
pub(crate) const PHYSENV_SIG: &str =
    "48 8B 0D ?? ?? ?? ?? 4C 8D 85 ?? ?? ?? ?? 48 8B D6 4C 8B 09 41 FF 51 58";
#[cfg(feature = "server")]
#[cfg(not(any(windows, test)))]
pub(crate) const PHYSENV_SIG: &str = linux_sigs::PHYSENV_SIG;
#[cfg(feature = "server")]
#[cfg(any(windows, test))]
pub(crate) const CREATE_FLUID_SLOT: usize = 11;
#[cfg(feature = "server")]
/// Linux vtables have two destructor slots, so the later slots are one higher.
#[cfg(not(any(windows, test)))]
pub(crate) const CREATE_FLUID_SLOT: usize = 12;
#[cfg(feature = "server")]
#[cfg(any(windows, test))]
pub(crate) const DESTROY_FLUID_SLOT: usize = 12;
#[cfg(feature = "server")]
#[cfg(not(any(windows, test)))]
pub(crate) const DESTROY_FLUID_SLOT: usize = 13;

#[cfg(feature = "server")]
/// Address of the engine's `physenv` variable.
pub(crate) const unsafe fn physenv_global_from(func: *const u8) -> *const *mut c_void {
    unsafe {
        let disp = func.add(3).cast::<i32>().read_unaligned() as isize;
        func.offset(7 + disp).cast::<*mut c_void>()
    }
}

#[cfg(feature = "server")]
/// `fluidparams_t` (x64 layout).
#[repr(C)]
pub(crate) struct FluidParams {
    pub surface_plane: [f32; 4],
    pub current_velocity: [f32; 3],
    pub damping: f32,
    pub torque_factor: f32,
    pub viscosity_factor: f32,
    pub game_data: *mut c_void,
    pub use_aerodynamics: u8,
    pub contents: i32,
}

#[cfg(feature = "server")]
type CreateFluidFn = engine_fn!((*mut c_void, *mut c_void, *const FluidParams) -> *mut c_void);
#[cfg(feature = "server")]
type DestroyFluidFn = engine_fn!((*mut c_void, *mut c_void));

#[cfg(feature = "server")]
pub(crate) unsafe fn create_fluid(
    env: *mut c_void,
    object: *mut c_void,
    params: &FluidParams,
) -> *mut c_void {
    unsafe {
        let vtable = (env as *const *const usize).read();
        let f: CreateFluidFn = std::mem::transmute(vtable.add(CREATE_FLUID_SLOT).read());
        f(env, object, params)
    }
}

#[cfg(feature = "server")]
/// `IPhysicsObject` vtable slots (confirmed in the world water setup:
/// `call [rbx+0xb8]`, `or ax,0x200`, `call [rbx+0xb0]`).
#[cfg(any(windows, test))]
pub(crate) const OBJ_SET_CALLBACK_FLAGS_SLOT: usize = 0xb0 / 8;
#[cfg(feature = "server")]
#[cfg(not(any(windows, test)))]
pub(crate) const OBJ_SET_CALLBACK_FLAGS_SLOT: usize = 0xb8 / 8;
#[cfg(feature = "server")]
#[cfg(any(windows, test))]
pub(crate) const OBJ_GET_CALLBACK_FLAGS_SLOT: usize = 0xb8 / 8;
#[cfg(feature = "server")]
#[cfg(not(any(windows, test)))]
pub(crate) const OBJ_GET_CALLBACK_FLAGS_SLOT: usize = 0xc0 / 8;
#[cfg(feature = "server")]
/// The engine ORs this into the world water object before creating its
/// fluid controller (`CALLBACK_GLOBAL_TOUCH_STATIC`, unconfirmed name).
pub(crate) const FLUID_OBJECT_CALLBACK_FLAG: u16 = 0x200;

#[cfg(feature = "server")]
type GetFlagsFn = engine_fn!((*mut c_void) -> u16);
#[cfg(feature = "server")]
type SetFlagsFn = engine_fn!((*mut c_void, u16));

#[cfg(feature = "server")]
/// Mirrors the engine's setup of a water object. Returns the old flags.
pub(crate) unsafe fn add_fluid_callback_flag(object: *mut c_void) -> u16 {
    unsafe {
        let vtable = (object as *const *const usize).read();
        let get: GetFlagsFn = std::mem::transmute(vtable.add(OBJ_GET_CALLBACK_FLAGS_SLOT).read());
        let set: SetFlagsFn = std::mem::transmute(vtable.add(OBJ_SET_CALLBACK_FLAGS_SLOT).read());
        let old = get(object);
        set(object, old | FLUID_OBJECT_CALLBACK_FLAG);
        old
    }
}

#[cfg(feature = "server")]
pub(crate) unsafe fn destroy_fluid(env: *mut c_void, fluid: *mut c_void) {
    unsafe {
        let vtable = (env as *const *const usize).read();
        let f: DestroyFluidFn = std::mem::transmute(vtable.add(DESTROY_FLUID_SLOT).read());
        f(env, fluid);
    }
}

// ---- CCollisionEvent::FluidEndTouch(IPhysicsObject *, IPhysicsFluidController *) --
//
// A physics object remembers the fluids it is touching, and when it is
// destroyed vphysics calls FluidEndTouch for each of them, even for a
// fluid controller that was destroyed in the meantime (our DetachFluid).
// The splash code in there reads the dead controller and crashed the
// game: removing a water volume (or the prop in it) while a prop was
// inside. The hook recognises the controllers this module destroyed and
// does the rest of FluidEndTouch without them: it clears
// EFL_TOUCHING_FLUID and sends ENTITY_EVENT_WATER_UNTOUCH, no splash.
// (The signature covers the entity offsets and slots used here.)

#[cfg(feature = "server")]
#[cfg(any(windows, test))]
pub(crate) const FLUID_END_TOUCH_SIG: &str = "48 89 5C 24 10 48 89 7C 24 18 41 56 48 83 EC 20 \
    FF 81 ?? ?? 00 00 4D 8B F0 48 8B DA 48 8B F9 48 85 D2 0F 84 ?? ?? ?? ?? 4D 85 C0 74 ?? \
    48 8B 02 48 8B CA 48 89 74 24 30 FF 90 88 00 00 00 48 8B F0 48 85 C0 74 ?? \
    48 8B D0 48 8B CF E8 ?? ?? ?? ?? 0F 2F 05 ?? ?? ?? ?? 72 ?? 4C 8B C6 48 8B D3 49 8B CE \
    E8 ?? ?? ?? ?? 81 A6 ?? ?? 00 00 FF FF F7 FF 49 8B CE 49 8B 06 48 8B 1E FF 50 30 \
    4C 63 C0 BA 01 00 00 00 48 8B CE FF 93 ?? ?? 00 00 FF 8F ?? ?? 00 00";
/// Where `m_iEFlags`'s offset (`and [rsi+disp32], ~EFL_TOUCHING_FLUID`) and the
/// `OnEntityEvent` vtable offset (`call [rbx+disp32]`) sit in the match of `FLUID_END_TOUCH_SIG`.
#[cfg(all(feature = "server", any(windows, test)))]
pub(crate) const FLUID_EFLAGS_DISP_AT: usize = 106;
#[cfg(all(feature = "server", any(windows, test)))]
pub(crate) const FLUID_EVENT_DISP_AT: usize = 139;
#[cfg(feature = "server")]
#[cfg(not(any(windows, test)))]
pub(crate) const FLUID_END_TOUCH_SIG: &str = linux_sigs::FLUID_END_TOUCH_SIG;
#[cfg(feature = "server")]
/// Whole instructions covered by the 14-byte jump.
#[cfg(any(windows, test))]
pub(crate) const FLUID_END_TOUCH_STOLEN: usize = 16;
#[cfg(feature = "server")]
#[cfg(not(any(windows, test)))]
pub(crate) const FLUID_END_TOUCH_STOLEN: usize = 14;
#[cfg(feature = "server")]
pub(crate) static FLUID_END_TOUCH_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);
#[cfg(feature = "server")]
/// `IPhysicsObject::GetGameData` (the entity), vtable offset 0x88.
#[cfg(any(windows, test))]
pub(crate) const OBJ_GET_GAME_DATA_SLOT: usize = 0x88 / 8;
#[cfg(feature = "server")]
#[cfg(not(any(windows, test)))]
pub(crate) const OBJ_GET_GAME_DATA_SLOT: usize = 0x90 / 8;
#[cfg(feature = "server")]
/// `CBaseEntity::m_iEFlags` and its `EFL_TOUCHING_FLUID` bit.
#[cfg(any(windows, test))]
pub(crate) const ENTITY_EFLAGS_OFFSET: usize = 0x190;
#[cfg(feature = "server")]
#[cfg(not(any(windows, test)))]
pub(crate) const ENTITY_EFLAGS_OFFSET: usize = 0x1b8;
#[cfg(feature = "server")]
pub(crate) const EFL_TOUCHING_FLUID: u32 = 0x80000;
#[cfg(feature = "server")]
/// `CBaseEntity::OnEntityEvent(event`, data), vtable offset 0x2e8.
#[cfg(any(windows, test))]
pub(crate) const ENTITY_EVENT_SLOT: usize = 0x2e8 / 8;
#[cfg(feature = "server")]
#[cfg(not(any(windows, test)))]
pub(crate) const ENTITY_EVENT_SLOT: usize = 0x2f0 / 8;
#[cfg(feature = "server")]
pub(crate) const ENTITY_EVENT_WATER_UNTOUCH: c_int = 1;

#[cfg(feature = "server")]
/// Controllers destroyed by this module (address, contents) that
/// vphysics objects may still point at.
static DEAD_FLUIDS: Mutex<Vec<(usize, c_int)>> = Mutex::new(Vec::new());
#[cfg(feature = "server")]
const DEAD_FLUIDS_MAX: usize = 256;

#[cfg(feature = "server")]
pub(crate) fn fluid_died(controller: usize, contents: c_int) {
    let mut dead = DEAD_FLUIDS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    dead.retain(|d| d.0 != controller);
    if dead.len() >= DEAD_FLUIDS_MAX {
        dead.remove(0);
    }
    dead.push((controller, contents));
}

#[cfg(feature = "server")]
/// A new controller may reuse a dead one's address.
pub(crate) fn fluid_born(controller: usize) {
    DEAD_FLUIDS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .retain(|d| d.0 != controller);
}

#[cfg(feature = "server")]
pub(crate) fn forget_dead_fluids() {
    DEAD_FLUIDS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clear();
}

engine_abi_item! {
#[cfg(feature = "server")]
pub(crate) unsafe fn hook_fluid_end_touch(
    this: *mut c_void,
    object: *mut c_void,
    fluid: *mut c_void,
) {
    unsafe {
        // Never block inside an engine callback.
        let dead = DEAD_FLUIDS
            .try_lock()
            .ok()
            .and_then(|d| d.iter().find(|e| e.0 == fluid as usize).map(|e| e.1));
        let Some(contents) = dead else {
            let original: engine_fn!((*mut c_void, *mut c_void, *mut c_void)) =
                std::mem::transmute(FLUID_END_TOUCH_TRAMPOLINE.load(Ordering::Relaxed));
            original(this, object, fluid);
            return;
        };
        if object.is_null() {
            return;
        }
        let get_game_data: engine_fn!((*mut c_void) -> *mut u8) =
            std::mem::transmute(
                *(*(object as *const *const usize)).add(OBJ_GET_GAME_DATA_SLOT),
            );
        let entity = get_game_data(object);
        if entity.is_null() {
            return;
        }
        let flags = entity.add(EFLAGS_OFFSET.load(Ordering::Relaxed)).cast::<u32>();
        flags.write_unaligned(flags.read_unaligned() & !EFL_TOUCHING_FLUID);
        let on_event: engine_fn!((*mut u8, c_int, *mut c_void)) =
            std::mem::transmute(
                *(*(entity as *const *const usize)).add(EVENT_SLOT.load(Ordering::Relaxed)),
            );
        on_event(
            entity,
            ENTITY_EVENT_WATER_UNTOUCH,
            contents as isize as *mut c_void,
        );
    }
}
}

// ---- UTIL_BubbleTrail(const Vector &from, const Vector &to, int count) --
//
// Found in the 64-bit server.dll by signature (5 call sites; it is the
// only user of the bubble sprite index that takes a from/to pair). It asks
// for the water height above both end points with a downward MASK_WATER
// ray (the ray helper), clips the trail by that *depth* only, and
// sends the whole remaining segment to clients as one temp entity with
// `count` bubbles. Nothing limits the trail sideways, so a shot that
// leaves a volume through a side wall produces a trail as long as the
// bullet's flight. In a map the pool walls do that job; a volume has none.
//
// The hook clips the segment to the volume it is in (or enters) first and
// scales `count` by the kept fraction; everything else is left alone.

#[cfg(feature = "server")]
#[cfg(any(windows, test))]
pub(crate) const BUBBLE_TRAIL_SIG: &str =
    "48 8B C4 48 89 58 08 48 89 70 10 57 48 81 EC E0 00 00 00 \
                                    F3 0F 10 49 08 41 8B F0 0F 29 70 E8 0F 28 D1 \
                                    F3 0F 58 15 ?? ?? ?? ?? 0F 29 78 D8 48 8B FA \
                                    44 0F 29 48 B8 48 8B D9 E8";
#[cfg(feature = "server")]
#[cfg(not(any(windows, test)))]
pub(crate) const BUBBLE_TRAIL_SIG: &str = linux_sigs::BUBBLE_TRAIL_SIG;
#[cfg(feature = "server")]
/// Whole instructions covered by the 14-byte jump.
#[cfg(any(windows, test))]
pub(crate) const BUBBLE_TRAIL_STOLEN: usize = 19;
#[cfg(feature = "server")]
#[cfg(not(any(windows, test)))]
pub(crate) const BUBBLE_TRAIL_STOLEN: usize = 15;
#[cfg(feature = "server")]
pub(crate) static BUBBLE_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);
#[cfg(feature = "server")]
pub(crate) static BUBBLE_CLIPS: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "server")]
/// A detour of arbitrary stolen length.
pub(crate) struct Patch {
    pub target: usize,
    pub original: Vec<u8>,
    pub trampoline: usize,
}

#[cfg(feature = "server")]
/// Detours `target` (first `stolen` bytes must be whole, position
/// independent instructions and match `sig`). The trampoline address is
/// returned in the patch.
pub(crate) unsafe fn install_patch(
    target: *mut u8,
    sig: &str,
    stolen: usize,
    hook: usize,
    p: &Platform,
) -> Result<Patch, &'static str> {
    unsafe { install_patch_relocating(target, sig, stolen, &[], hook, p) }
}

#[cfg(feature = "server")]
/// `lea r64, [rip+disp32]` (7 bytes) at `at` in `code`, rewritten as `movabs r64, imm64`
/// (10 bytes) that loads the same address when the code runs from somewhere else.
pub(crate) fn relocate_rip_lea(code: &[u8], at: usize, origin: u64) -> Option<[u8; 10]> {
    let b = code.get(at..at + 7)?;
    if (b[0] & 0xFB) != 0x48 || b[1] != 0x8D || (b[2] & 0xC7) != 0x05 {
        return None;
    }
    let disp = i64::from(i32::from_le_bytes(b[3..7].try_into().ok()?));
    let value = (origin + at as u64 + 7).wrapping_add_signed(disp);
    let reg = ((b[0] >> 2) & 1) << 3 | ((b[2] >> 3) & 7);
    let mut out = [0u8; 10];
    out[0] = 0x48 | (reg >> 3);
    out[1] = 0xB8 | (reg & 7);
    out[2..].copy_from_slice(&value.to_le_bytes());
    Some(out)
}

#[cfg(feature = "server")]
/// Like [`install_patch`], for a prologue that holds `lea r64,[rip+disp32]` instructions
/// (at the byte offsets in `rip_leas`): those are rewritten in the trampoline.
pub(crate) unsafe fn install_patch_relocating(
    target: *mut u8,
    sig: &str,
    stolen: usize,
    rip_leas: &[usize],
    hook: usize,
    p: &Platform,
) -> Result<Patch, &'static str> {
    unsafe {
        assert!(stolen >= 14);
        let pat = parse_sig(sig);
        let original = std::slice::from_raw_parts(target, stolen).to_vec();
        for (i, want) in pat.iter().take(stolen).enumerate() {
            if let Some(w) = want {
                if original[i] != *w {
                    return Err("function prologue does not match");
                }
            }
        }
        let tramp = (p.alloc_exec)(64);
        if tramp.is_null() {
            return Err("could not allocate executable memory");
        }
        let mut tb = Vec::with_capacity(stolen + 32);
        let mut i = 0;
        while i < stolen {
            if rip_leas.contains(&i) {
                let Some(abs) = relocate_rip_lea(&original, i, target as u64) else {
                    return Err("prologue lea cannot be relocated");
                };
                tb.extend_from_slice(&abs);
                i += 7;
            } else {
                tb.push(original[i]);
                i += 1;
            }
        }
        tb.extend_from_slice(&[0xFF, 0x25, 0, 0, 0, 0]);
        tb.extend_from_slice(&(target as u64 + stolen as u64).to_le_bytes());
        std::ptr::copy_nonoverlapping(tb.as_ptr(), tramp, tb.len());
        (p.flush)(tramp, tb.len());

        let mut detour = vec![0x90u8; stolen];
        detour[0] = 0xFF;
        detour[1] = 0x25;
        detour[2..6].copy_from_slice(&0u32.to_le_bytes()); // jmp [rip+0]
        detour[6..14].copy_from_slice(&(hook as u64).to_le_bytes());
        let Some(old) = (p.make_writable)(target, stolen) else {
            return Err("could not make the function writable");
        };
        std::ptr::copy_nonoverlapping(detour.as_ptr(), target, stolen);
        (p.restore_protect)(target, stolen, old);
        (p.flush)(target, stolen);
        Ok(Patch {
            target: target as usize,
            original,
            trampoline: tramp as usize,
        })
    }
}

#[cfg(feature = "server")]
pub(crate) unsafe fn uninstall_patch(patch: &Patch, p: &Platform) {
    unsafe {
        let target = patch.target as *mut u8;
        if let Some(old) = (p.make_writable)(target, patch.original.len()) {
            std::ptr::copy_nonoverlapping(patch.original.as_ptr(), target, patch.original.len());
            (p.restore_protect)(target, patch.original.len(), old);
            (p.flush)(target, patch.original.len());
        }
    }
}

#[cfg(feature = "server")]
/// Parameter interval `[t0, t1]` (clamped to 0..1) in which the segment
/// `s + t*d` lies inside the box. `None` if it never does.
pub(crate) fn clip_segment_to_box(
    s: [f32; 3],
    d: [f32; 3],
    lo: [f32; 3],
    hi: [f32; 3],
) -> Option<(f32, f32)> {
    let mut t0 = 0.0f32;
    let mut t1 = 1.0f32;
    for i in 0..3 {
        if d[i].abs() < 1e-9 {
            if s[i] < lo[i] || s[i] > hi[i] {
                return None;
            }
            continue;
        }
        let a = (lo[i] - s[i]) / d[i];
        let b = (hi[i] - s[i]) / d[i];
        let (near, far) = if a < b { (a, b) } else { (b, a) };
        t0 = t0.max(near);
        t1 = t1.min(far);
        if t0 > t1 {
            return None;
        }
    }
    Some((t0, t1))
}

#[cfg(feature = "server")]
#[derive(Debug, PartialEq)]
pub(crate) enum BubbleClip {
    /// Call the engine function unchanged.
    Keep,
    /// Nothing worth drawing: skip the call.
    Skip,
    /// Call it with these end points and the count scaled by the fraction.
    Clipped([f32; 3], [f32; 3], f32),
}

#[cfg(feature = "server")]
pub(crate) fn clip_bubble_trail(vols: &[Volume], from: [f32; 3], to: [f32; 3]) -> BubbleClip {
    let d = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
    let len = d[2].mul_add(d[2], d[1].mul_add(d[1], d[0] * d[0])).sqrt();
    if len.is_nan() || len <= 1e-3 {
        return BubbleClip::Keep;
    }
    // A volume the trail starts in wins; otherwise the first one it enters.
    let mut best: Option<(f32, f32)> = None;
    let mut starts_inside = false;
    for v in vols {
        let Some((t0, t1)) = clip_segment_to_box(from, d, v.min, v.max) else {
            continue;
        };
        if t0 <= 0.0 {
            if !starts_inside || t1 > best.map_or(0.0, |b| b.1) {
                best = Some((t0, t1));
                starts_inside = true;
            }
        } else if !starts_inside && best.map_or(true, |b| t0 < b.0) {
            best = Some((t0, t1));
        }
    }
    let Some((mut a, mut b)) = best else {
        return BubbleClip::Keep; // not in a volume: the engine's own rules apply
    };
    if a <= 0.0 && b >= 1.0 {
        return BubbleClip::Keep; // entirely inside
    }
    // Keep the new end points a hair inside so the engine's own depth
    // probe there still sees water.
    let eps = 0.5 / len;
    if a > 0.0 {
        a += eps;
    }
    if b < 1.0 {
        b -= eps;
    }
    if b <= a {
        return BubbleClip::Skip;
    }
    let at = |t: f32| {
        [
            d[0].mul_add(t, from[0]),
            d[1].mul_add(t, from[1]),
            d[2].mul_add(t, from[2]),
        ]
    };
    BubbleClip::Clipped(at(a), at(b), b - a)
}

engine_abi_item! {
#[cfg(feature = "server")]
/// Replacement for `UTIL_BubbleTrail` (the engine's calling convention).
pub(crate) unsafe fn hook_bubble_trail(
    from: *const f32,
    to: *const f32,
    count: c_int,
) {
    unsafe {
        let original: engine_fn!((*const f32, *const f32, c_int)) =
            std::mem::transmute(BUBBLE_TRAMPOLINE.load(Ordering::Relaxed));
        let mut action = BubbleClip::Keep;
        if !from.is_null() && !to.is_null() {
            let f = [
                from.read_unaligned(),
                from.add(1).read_unaligned(),
                from.add(2).read_unaligned(),
            ];
            let t = [
                to.read_unaligned(),
                to.add(1).read_unaligned(),
                to.add(2).read_unaligned(),
            ];
            // A trail that stays clear of every volume is kept as it is, without a copy
            // of the snapshot. No lock is held when the original is called (it calls
            // back into the point-contents hook).
            let d = [t[0] - f[0], t[1] - f[1], t[2] - f[2]];
            if let Some(vols) =
                snapshot_if(|v| !segment_misses_bounds(v.bounds, f, d, [0.0; 3]))
            {
                action = clip_bubble_trail(&vols, f, t);
            }
        }
        match action {
            BubbleClip::Keep => original(from, to, count),
            BubbleClip::Skip => {
                bump(&BUBBLE_CLIPS);
            }
            BubbleClip::Clipped(a, b, frac) => {
                bump(&BUBBLE_CLIPS);
                let n = if count > 0 {
                    ((count as f32 * frac).ceil() as c_int).max(1)
                } else {
                    count
                };
                original(a.as_ptr(), b.as_ptr(), n);
            }
        }
    }
}
}

// ---- Explosions ------------------------------------------------------
//
// Several explosive entities choose between the normal explosion and the
// engine's water one (the "WaterSurfaceExplosion" effect: underwater
// sound, splash and wake, all made by the client from that one effect)
// by looking at their own m_nWaterLevel (byte at this+0x2ab), not at
// point contents:
//   * 0x5ae2f0  needs a water level of 2 or more ("deep enough")
//   * 0x5c9540 and 0x6df9d0  need any water level
// (+0x33c is the entity's absolute origin.) An entity whose origin is in a
// volume gets its water level raised to 2 before the original runs.

#[cfg(feature = "server")]
/// Per function: the prologue of each known build (tried in order).
/// Newer builds store rbx at [rsp+0x10] and reshuffle the pushes.
#[cfg(any(windows, test))]
pub(crate) const EXPLOSION_TARGETS: [&[(&str, usize)]; 3] = [
    &[
        ("48 89 5C 24 08 55 48 8D 6C 24 A9 48 81 EC F0 00 00 00 80 B9 ?? 02 00 00 02", 18),
        ("48 89 5C 24 10 55 48 8D 6C 24 A9 48 81 EC F0 00 00 00 80 B9 ?? 02 00 00 02", 18),
    ],
    &[
        ("48 8B C4 48 89 58 08 48 89 70 10 48 89 78 18 4C 89 70 20 55 48 8D 68 D8 48 81 EC 20 01 00 00 0F 29 70 E8 48 8B D9 8B 89 ?? 02 00 00", 19),
        ("48 8B C4 48 89 58 10 48 89 70 18 55 57 41 56 48 8D 68 C8 48 81 EC 20 01 00 00 0F 29 70 D8 48 8B D9 8B 89 ?? 02 00 00", 15),
    ],
    &[
        ("48 89 5C 24 08 48 89 7C 24 10 55 48 8D 6C 24 A9 48 81 EC F0 00 00 00 80 B9 ?? 02 00 00 00", 16),
        ("48 89 5C 24 10 48 89 7C 24 18 55 48 8D 6C 24 A9 48 81 EC F0 00 00 00 80 B9 ?? 02 00 00 00", 16),
    ],
];
#[cfg(feature = "server")]
#[cfg(not(any(windows, test)))]
pub(crate) const EXPLOSION_TARGETS: [&[(&str, usize)]; 3] = linux_sigs::EXPLOSION_TARGETS;

#[cfg(feature = "server")]
/// The first alternative of target `i` that matches exactly once.
pub(crate) unsafe fn find_explosion_target(
    base: *const u8,
    i: usize,
) -> Result<(*mut u8, &'static str, usize), &'static str> {
    unsafe {
        let mut last = "signature not found (different server.dll build?)";
        for (sig, stolen) in EXPLOSION_TARGETS[i] {
            match scan_sig(base, sig) {
                Ok(at) => return Ok((at, *sig, *stolen)),
                Err(e) => last = e,
            }
        }
        Err(last)
    }
}
#[cfg(feature = "server")]
pub(crate) static EXPLOSION_TRAMPOLINES: [AtomicUsize; 3] = [
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
];
#[cfg(feature = "server")]
pub(crate) static EXPLOSION_FIXES: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "server")]
#[cfg(any(windows, test))]
pub(crate) const ENTITY_WATER_LEVEL_OFFSET: usize = 0x2ab;
#[cfg(feature = "server")]
#[cfg(not(any(windows, test)))]
pub(crate) const ENTITY_WATER_LEVEL_OFFSET: usize = 0x2d3;
#[cfg(feature = "server")]
#[cfg(any(windows, test))]
pub(crate) const ENTITY_ORIGIN_OFFSET: usize = 0x33c;
#[cfg(feature = "server")]
#[cfg(not(any(windows, test)))]
pub(crate) const ENTITY_ORIGIN_OFFSET: usize = 0x364;

// The entity layout of the running game build. It differs between builds (the dev branch's
// Windows server has everything 8 bytes further on than the default branch's), so the
// Windows builds read it from the signature matches (`learn_*`); until then, and always on
// Linux, the constants above are used.
#[cfg(feature = "server")]
pub(crate) static WATER_LEVEL_OFFSET: AtomicUsize = AtomicUsize::new(ENTITY_WATER_LEVEL_OFFSET);
#[cfg(feature = "server")]
pub(crate) static ORIGIN_OFFSET: AtomicUsize = AtomicUsize::new(ENTITY_ORIGIN_OFFSET);
#[cfg(feature = "server")]
pub(crate) static EFLAGS_OFFSET: AtomicUsize = AtomicUsize::new(ENTITY_EFLAGS_OFFSET);
#[cfg(feature = "server")]
pub(crate) static EVENT_SLOT: AtomicUsize = AtomicUsize::new(ENTITY_EVENT_SLOT);
/// `m_vecAbsOrigin` is this far after `m_nWaterLevel` in `CBaseEntity` (0x2ab -> 0x33c in the
/// default Windows build, 0x2d3 -> 0x364 in the dev Linux build).
#[cfg(feature = "server")]
pub(crate) const ORIGIN_AFTER_WATER_LEVEL: usize = 0x91;

#[cfg(feature = "server")]
#[allow(clippy::missing_const_for_fn)]
pub(crate) unsafe fn read_disp(at: *const u8, i: usize) -> usize {
    unsafe { at.add(i).cast::<u32>().read_unaligned() as usize }
}

/// Reads `m_nWaterLevel`'s offset out of a match of `SIG` (the four places that name it
/// must agree) and derives where the origin is.
#[cfg(all(feature = "server", any(windows, test)))]
pub(crate) unsafe fn learn_water_level_layout(at: *const u8) -> Result<usize, &'static str> {
    unsafe {
        let off = read_disp(at, WATER_LEVEL_DISP_AT[0]);
        if !(0x100..0x1000).contains(&off)
            || WATER_LEVEL_DISP_AT.iter().any(|&i| read_disp(at, i) != off)
        {
            return Err("the entity layout is not the expected one (different game build?)");
        }
        WATER_LEVEL_OFFSET.store(off, Ordering::SeqCst);
        ORIGIN_OFFSET.store(off + ORIGIN_AFTER_WATER_LEVEL, Ordering::SeqCst);
        Ok(off)
    }
}

#[cfg(all(feature = "server", not(any(windows, test))))]
#[allow(clippy::missing_const_for_fn, clippy::unnecessary_wraps)]
pub(crate) unsafe fn learn_water_level_layout(_at: *const u8) -> Result<usize, &'static str> {
    Ok(ENTITY_WATER_LEVEL_OFFSET)
}

/// Reads `m_iEFlags`' offset and the `OnEntityEvent` vtable slot out of a match of
/// `FLUID_END_TOUCH_SIG`.
#[cfg(all(feature = "server", any(windows, test)))]
pub(crate) unsafe fn learn_fluid_layout(at: *const u8) -> Result<(), &'static str> {
    unsafe {
        let flags = read_disp(at, FLUID_EFLAGS_DISP_AT);
        let event = read_disp(at, FLUID_EVENT_DISP_AT);
        if !(0x100..0x1000).contains(&flags) || event % 8 != 0 || !(0x100..0x1000).contains(&event)
        {
            return Err("the entity layout is not the expected one (different game build?)");
        }
        EFLAGS_OFFSET.store(flags, Ordering::SeqCst);
        EVENT_SLOT.store(event / 8, Ordering::SeqCst);
        Ok(())
    }
}

#[cfg(all(feature = "server", not(any(windows, test))))]
#[allow(clippy::missing_const_for_fn, clippy::unnecessary_wraps)]
pub(crate) unsafe fn learn_fluid_layout(_at: *const u8) -> Result<(), &'static str> {
    Ok(())
}

#[cfg(feature = "server")]
/// Raises `this`'s water level to 2 if its origin is in a volume.
pub(crate) unsafe fn raise_water_level_in_volume(this: *mut u8) {
    unsafe {
        if this.is_null() {
            return;
        }
        let o = this.add(ORIGIN_OFFSET.load(Ordering::Relaxed)) as *const f32;
        let p = [
            o.read_unaligned(),
            o.add(1).read_unaligned(),
            o.add(2).read_unaligned(),
        ];
        // try_lock: never wait (the original calls back into the contents hook).
        let inside = snapshot_if(|v| v.may_contain(p)).is_some_and(|v| v.contains(p));
        if inside {
            let level = this.add(WATER_LEVEL_OFFSET.load(Ordering::Relaxed));
            if level.read() < 2 {
                level.write(2);
                bump(&EXPLOSION_FIXES);
            }
        }
    }
}

#[cfg(feature = "server")]
type ExplosionFn = engine_fn!((*mut u8, usize, usize, usize) -> usize);

#[cfg(feature = "server")]
unsafe fn explosion_hook(i: usize, this: *mut u8, a: usize, b: usize, c: usize) -> usize {
    unsafe {
        raise_water_level_in_volume(this);
        let original: ExplosionFn =
            std::mem::transmute(EXPLOSION_TRAMPOLINES[i].load(Ordering::Relaxed));
        original(this, a, b, c)
    }
}
engine_abi_item! {
#[cfg(feature = "server")]
pub(crate) unsafe fn hook_explosion0(
    t: *mut u8,
    a: usize,
    b: usize,
    c: usize,
) -> usize {
    unsafe { explosion_hook(0, t, a, b, c) }
}
}
engine_abi_item! {
#[cfg(feature = "server")]
pub(crate) unsafe fn hook_explosion1(
    t: *mut u8,
    a: usize,
    b: usize,
    c: usize,
) -> usize {
    unsafe { explosion_hook(1, t, a, b, c) }
}
}
engine_abi_item! {
#[cfg(feature = "server")]
pub(crate) unsafe fn hook_explosion2(
    t: *mut u8,
    a: usize,
    b: usize,
    c: usize,
) -> usize {
    unsafe { explosion_hook(2, t, a, b, c) }
}
}
#[cfg(feature = "server")]
pub(crate) const EXPLOSION_HOOKS: [ExplosionFn; 3] =
    [hook_explosion0, hook_explosion1, hook_explosion2];

#[cfg(feature = "server")]
/// DispatchEffect(const char *name, const `CEffectData` &data).
/// Only its address is used (to send the water explosion effect), nothing is patched.
#[cfg(any(windows, test))]
pub(crate) const DISPATCH_EFFECT_SIG: &str = "48 89 5C 24 08 57 48 83 EC 70 48 8B D9 48 8B FA \
                                       48 8D 4C 24 30 E8 ?? ?? ?? ?? 48 8D 05 ?? ?? ?? ?? 48 8B D7";
#[cfg(feature = "server")]
#[cfg(not(any(windows, test)))]
pub(crate) const DISPATCH_EFFECT_SIG: &str = linux_sigs::DISPATCH_EFFECT_SIG;
#[cfg(feature = "server")]
#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
pub(crate) const DISPATCH_EFFECT_STOLEN: usize = 16;

// ---- Explosion events (CTEExplosion::Create) -------------------------
//
// Most explosives (barrels, RPG, grenades, SLAM ...) send the client an
// explosion *event*, and the client decides for itself, with its own
// point-contents check, whether to play the normal explosion or the water
// one (underwater sound, splash, wake). A volume does not exist for that
// check. The one thing the server can do is what the engine's own water
// explosion code does: send the "WaterSurfaceExplosion" effect instead.
// So CTEExplosion's Create (vtable slot 4) is swapped: an explosion whose
// origin (this+0x18) is in a volume is sent as that effect, and the event
// is dropped.

#[cfg(feature = "server")]
pub(crate) const TE_ORIGIN_OFFSET: usize = 0x18;
/// `CTEExplosion` fields: scale (f32), flags (u32), magnitude (i32).
#[cfg(feature = "server")]
#[cfg(any(windows, test))]
pub(crate) const TE_SCALE_OFFSET: usize = 0x28;
#[cfg(feature = "server")]
#[cfg(any(windows, test))]
pub(crate) const TE_FLAGS_OFFSET: usize = 0x2c;
#[cfg(feature = "server")]
#[cfg(any(windows, test))]
pub(crate) const TE_MAGNITUDE_OFFSET: usize = 0x34;
#[cfg(feature = "server")]
#[cfg(not(any(windows, test)))]
pub(crate) const TE_SCALE_OFFSET: usize = 0x24;
#[cfg(feature = "server")]
#[cfg(not(any(windows, test)))]
pub(crate) const TE_FLAGS_OFFSET: usize = 0x28;
#[cfg(feature = "server")]
#[cfg(not(any(windows, test)))]
pub(crate) const TE_MAGNITUDE_OFFSET: usize = 0x30;
/// Offsets in the `Create` prologue of `lea r64,[rip+disp32]` instructions.
#[cfg(feature = "server")]
#[cfg(any(windows, test))]
pub(crate) const TE_CREATE_RIP_LEAS: &[usize] = &[];
#[cfg(feature = "server")]
#[cfg(not(any(windows, test)))]
pub(crate) const TE_CREATE_RIP_LEAS: &[usize] = &[8];
/// `CBaseTempEntity::m_pszName` (a `const char*`).
#[cfg(feature = "server")]
#[cfg(not(any(windows, test)))]
const TE_NAME_OFFSET: usize = 8;

/// True for a `CTEExplosion`: Windows compares the vtable (found through RTTI); Linux has
/// no RTTI for it and compares the temp entity's name, "Explosion".
#[cfg(feature = "server")]
#[cfg(any(windows, test))]
unsafe fn is_te_explosion(this: *const u8) -> bool {
    unsafe {
        let vtable = TE_EXPLOSION_VTABLE.load(Ordering::Relaxed);
        vtable != 0 && this.cast::<usize>().read() == vtable
    }
}
#[cfg(feature = "server")]
#[cfg(not(any(windows, test)))]
unsafe fn is_te_explosion(this: *const u8) -> bool {
    unsafe {
        if TE_EXPLOSION_VTABLE.load(Ordering::Relaxed) == 0 {
            return false;
        }
        let name = this
            .add(TE_NAME_OFFSET)
            .cast::<*const u8>()
            .read_unaligned();
        !name.is_null() && CStr::from_ptr(name.cast()).to_bytes() == b"Explosion"
    }
}
#[cfg(feature = "server")]
pub(crate) static ORIGINAL_TE_CREATE: AtomicUsize = AtomicUsize::new(0);
#[cfg(feature = "server")]
pub(crate) static DISPATCH_EFFECT_ADDR: AtomicUsize = AtomicUsize::new(0);
#[cfg(feature = "server")]
pub(crate) static EXPLOSIONS_REDIRECTED: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "server")]
#[cfg(not(any(windows, test)))]
/// The Linux server has no RTTI for `CTEExplosion`, so no vtable is looked up: the hook
/// recognises the object by its name instead ([`is_te_explosion`]). The returned `1` is
/// only the "armed" marker stored in [`TE_EXPLOSION_VTABLE`].
#[allow(clippy::unnecessary_wraps)] // same signature as the Windows lookup, which can fail
pub(crate) const unsafe fn te_explosion_vtable(_base: *const u8) -> Result<usize, &'static str> {
    Ok(1)
}

#[cfg(feature = "server")]
#[cfg(any(windows, test))]
/// Address of `CTEExplosion`'s vtable, found through RTTI only: the type
/// descriptor name, the complete object locator that names it, and the
/// pointer to that locator just before the vtable. No code bytes are
/// involved, so it does not depend on how the compiler laid out any
/// function.
pub(crate) unsafe fn te_explosion_vtable(base: *const u8) -> Result<usize, &'static str> {
    unsafe { rtti_vtable(base, b".?AVCTEExplosion@@\0") }
}

#[cfg(feature = "server")]
#[cfg(any(windows, test))]
pub(crate) unsafe fn rtti_vtable(base: *const u8, name: &[u8]) -> Result<usize, &'static str> {
    unsafe {
        let rd32 = |off: usize| base.add(off).cast::<u32>().read_unaligned();
        let rd16 = |off: usize| base.add(off).cast::<u16>().read_unaligned();
        if rd16(0) != 0x5A4D {
            return Err("module has no MZ header");
        }
        let nt = rd32(0x3C) as usize;
        let preferred = base.add(nt + 24 + 24).cast::<u64>().read_unaligned() as usize;
        let nsec = rd16(nt + 6) as usize;
        let sections = nt + 24 + rd16(nt + 20) as usize;
        let mut data: Vec<(usize, &[u8])> = Vec::new(); // (rva, bytes) of non-code sections
        for i in 0..nsec {
            let sh = sections + i * 40;
            let vsize = rd32(sh + 8) as usize;
            let va = rd32(sh + 12) as usize;
            if rd32(sh + 36) & 0x2000_0000 == 0 && rd32(sh + 36) & 0x4000_0000 != 0 {
                data.push((va, std::slice::from_raw_parts(base.add(va), vsize)));
            }
        }
        let find = |needle: &[u8], align: usize| -> Vec<usize> {
            let mut out = Vec::new();
            for (va, bytes) in &data {
                let mut i = 0;
                while i + needle.len() <= bytes.len() {
                    if &bytes[i..i + needle.len()] == needle && (va + i) % align == 0 {
                        out.push(va + i);
                    }
                    i += 1;
                }
            }
            out
        };
        let names = find(name, 1);
        let Some(&name_rva) = names.first() else {
            return Err("RTTI type name not found");
        };
        let td_rva = (name_rva - 16) as u32;
        for at in find(&td_rva.to_le_bytes(), 4) {
            if at < 12 {
                continue;
            }
            let col = at - 12;
            // complete object locator: signature 1 (x64), offset 0
            if rd32(col) != 1 || rd32(col + 4) != 0 {
                continue;
            }
            // `vtable[-1]` holds its address: relocated, or preferred-base when only mapped.
            for target in [base as usize + col, preferred + col] {
                if let Some(p) = find(&target.to_le_bytes(), 8).into_iter().next() {
                    return Ok(base as usize + p + 8);
                }
            }
        }
        Err("CTEExplosion vtable not found")
    }
}

#[cfg(feature = "server")]
/// Fills a `CEffectData` the way the engine does for the water explosion
/// (flags 0x428, scale 128, magnitude 50; byte at +0x88 set).
#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
pub(crate) fn water_explosion_effect_data(origin: [f32; 3]) -> [u8; 0xa0] {
    water_explosion_effect_data_with(origin, 0x428, 128.0, 50.0)
}

#[cfg(feature = "server")]
/// Same layout the client's own in-water path hands to the water explosion
/// (flags, scale and magnitude are the temp entity's). The client plays
/// WaterExplosionEffect.Sound only when scale > 128, else the base sound.
pub(crate) fn water_explosion_effect_data_with(
    origin: [f32; 3],
    flags: u32,
    scale: f32,
    magnitude: f32,
) -> [u8; 0xa0] {
    let mut d = [0u8; 0xa0];
    for (i, v) in origin.iter().enumerate() {
        d[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    d[0x30..0x34].copy_from_slice(&flags.to_le_bytes());
    d[0x38..0x3c].copy_from_slice(&scale.to_le_bytes());
    d[0x3c..0x40].copy_from_slice(&magnitude.to_le_bytes());
    d[0x88] = 1;
    d
}

#[cfg(feature = "server")]
/// True (and the effect sent) if the explosion at `this`' origin is in a volume.
pub(crate) unsafe fn redirect_explosion(this: *const u8) -> bool {
    unsafe {
        let dispatch = DISPATCH_EFFECT_ADDR.load(Ordering::Relaxed);
        if this.is_null() || dispatch == 0 {
            return false;
        }
        let o = this.add(TE_ORIGIN_OFFSET).cast::<f32>();
        let origin = [
            o.read_unaligned(),
            o.add(1).read_unaligned(),
            o.add(2).read_unaligned(),
        ];
        let inside =
            snapshot_if(|v| v.may_contain(origin)).is_some_and(|v| v.surface_z(origin).is_some());
        if !inside {
            return false;
        }
        // CTEExplosion fields (see TE_SCALE_OFFSET).
        // The effect gets the blast point: the client traces up from it to find
        // the surface and the depth (which picks the muffled sound).
        let scale = this.add(TE_SCALE_OFFSET).cast::<f32>().read_unaligned();
        let flags = this.add(TE_FLAGS_OFFSET).cast::<u32>().read_unaligned();
        let magnitude = this.add(TE_MAGNITUDE_OFFSET).cast::<i32>().read_unaligned() as f32;
        let data = water_explosion_effect_data_with(origin, flags, scale, magnitude);
        let f: engine_fn!((*const u8, *const u8)) = std::mem::transmute(dispatch);
        f(c"WaterSurfaceExplosion".as_ptr().cast(), data.as_ptr());
        bump(&EXPLOSIONS_REDIRECTED);
        true
    }
}

#[cfg(feature = "server")]
/// `CBaseTempEntity::Create(this`, filter, delay): every temp entity goes
/// through it (`CTempEnts::Explosion` jumps straight to it, so the vtable
/// slot is never used). Only `CTEExplosion` objects are looked at.
#[cfg(any(windows, test))]
pub(crate) const TE_CREATE_SIG: &str =
    "48 89 5C 24 08 48 89 6C 24 10 48 89 74 24 18 57 48 83 EC 40 \
                                 48 8B 05 ?? ?? ?? ?? 48 8B EA 0F 29 74 24 30";
#[cfg(feature = "server")]
#[cfg(not(any(windows, test)))]
pub(crate) const TE_CREATE_SIG: &str = linux_sigs::TE_CREATE_SIG;
#[cfg(feature = "server")]
#[cfg(any(windows, test))]
pub(crate) const TE_CREATE_STOLEN: usize = 20;
#[cfg(feature = "server")]
#[cfg(not(any(windows, test)))]
pub(crate) const TE_CREATE_STOLEN: usize = 15;
#[cfg(feature = "server")]
pub(crate) static TE_EXPLOSION_VTABLE: AtomicUsize = AtomicUsize::new(0);

engine_abi_item! {
#[cfg(feature = "server")]
pub(crate) unsafe fn hook_te_create(
    this: *mut u8,
    filter: *mut c_void,
    delay: f32,
) {
    unsafe {
        if !this.is_null() && is_te_explosion(this) && redirect_explosion(this) {
            return;
        }
        let original: engine_fn!((*mut u8, *mut c_void, f32)) =
            std::mem::transmute(ORIGINAL_TE_CREATE.load(Ordering::Relaxed));
        original(this, filter, delay);
    }
}
}

// ---- Windows glue (thin on purpose) ----------------------------------

// ---- Linux glue (thin on purpose) -------------------------------------

#[cfg(target_os = "linux")]
pub(crate) mod lin {
    #[allow(clippy::wildcard_imports)]
    use super::*;

    const PROT_READ: c_int = 1;
    const PROT_WRITE: c_int = 2;
    const PROT_EXEC: c_int = 4;
    const MAP_PRIVATE: c_int = 2;
    const MAP_ANONYMOUS: c_int = 0x20;
    const PAGE: usize = 4096;

    extern "C" {
        #[cfg(feature = "server")]
        fn mmap(
            addr: *mut c_void,
            len: usize,
            prot: c_int,
            flags: c_int,
            fd: c_int,
            off: i64,
        ) -> *mut c_void;
        fn mprotect(addr: *mut c_void, len: usize, prot: c_int) -> c_int;
    }

    #[cfg(feature = "server")]
    unsafe fn alloc_exec(size: usize) -> *mut u8 {
        unsafe {
            let p = mmap(
                std::ptr::null_mut(),
                size,
                PROT_READ | PROT_WRITE | PROT_EXEC,
                MAP_PRIVATE | MAP_ANONYMOUS,
                -1,
                0,
            );
            if p as isize == -1 {
                std::ptr::null_mut()
            } else {
                p.cast::<u8>()
            }
        }
    }

    /// Protection bits (`PROT_*`) of the mapping that holds `addr`.
    fn protection_of(addr: usize) -> Option<c_int> {
        let maps = std::fs::read_to_string("/proc/self/maps").ok()?;
        for line in maps.lines() {
            let mut it = line.split_whitespace();
            let (range, perms) = (it.next()?, it.next()?);
            let (lo, hi) = range.split_once('-')?;
            let (lo, hi) = (
                usize::from_str_radix(lo, 16).ok()?,
                usize::from_str_radix(hi, 16).ok()?,
            );
            if (lo..hi).contains(&addr) {
                let p = perms.as_bytes();
                return Some(
                    if p.first() == Some(&b'r') {
                        PROT_READ
                    } else {
                        0
                    } | if p.get(1) == Some(&b'w') {
                        PROT_WRITE
                    } else {
                        0
                    } | if p.get(2) == Some(&b'x') {
                        PROT_EXEC
                    } else {
                        0
                    },
                );
            }
        }
        None
    }

    fn page_span(p: *mut u8, n: usize) -> (*mut c_void, usize) {
        let start = p as usize & !(PAGE - 1);
        let end = (p as usize + n).div_ceil(PAGE) * PAGE;
        (start as *mut c_void, end - start)
    }

    unsafe fn make_writable(p: *mut u8, n: usize) -> Option<u32> {
        unsafe {
            let old = protection_of(p as usize)?;
            let (start, len) = page_span(p, n);
            (mprotect(start, len, old | PROT_WRITE) == 0).then_some(old as u32)
        }
    }
    unsafe fn restore_protect(p: *mut u8, n: usize, old: u32) {
        unsafe {
            let (start, len) = page_span(p, n);
            mprotect(start, len, old as c_int);
        }
    }
    #[cfg(feature = "server")]
    const unsafe fn flush(_p: *mut u8, _n: usize) {
        // x86 keeps instruction fetches coherent with stores
    }

    pub(crate) const PLATFORM: Platform = Platform {
        #[cfg(feature = "server")]
        alloc_exec,
        make_writable,
        restore_protect,
        #[cfg(feature = "server")]
        flush,
    };

    use crate::loaded_objects::{dl_iterate_phdr, DlPhdrInfo};

    /// Load address and full path of the loaded library whose path ends with one of
    /// `suffixes` (for example `/server_srv.so`).
    unsafe fn loaded_module(suffixes: &[&[u8]]) -> Option<(usize, CString)> {
        unsafe {
            struct Search<'a> {
                suffixes: &'a [&'a [u8]],
                found: Option<(usize, CString)>,
            }
            unsafe extern "C" fn find(
                info: *const DlPhdrInfo,
                _size: usize,
                data: *mut c_void,
            ) -> c_int {
                unsafe {
                    let search = &mut *data.cast::<Search<'_>>();
                    let name = (*info).name;
                    if name.is_null() {
                        return 0;
                    }
                    let name = CStr::from_ptr(name);
                    if search.suffixes.iter().any(|s| name.to_bytes().ends_with(s)) {
                        search.found = Some(((*info).addr, name.to_owned()));
                        return 1;
                    }
                    0
                }
            }
            let mut search = Search {
                suffixes,
                found: None,
            };
            dl_iterate_phdr(find, (&raw mut search).cast::<c_void>());
            search.found
        }
    }

    /// Load address of the 64-bit Linux server module (`server_srv.so`, or `server.so`).
    #[cfg(feature = "server")]
    pub(crate) unsafe fn server_base() -> *const u8 {
        unsafe {
            loaded_module(&[b"/server_srv.so", b"/server.so"])
                .map_or(std::ptr::null(), |(base, _)| base as *const u8)
        }
    }

    /// Load address of the 64-bit Linux client module (`client.so`).
    #[cfg(feature = "client")]
    pub(crate) unsafe fn client_base() -> *const u8 {
        unsafe {
            loaded_module(&[b"/client.so"]).map_or(std::ptr::null(), |(base, _)| base as *const u8)
        }
    }

    #[cfg(feature = "client")]
    /// An interface of client.so (`CreateInterface("VClientEntityList003")`).
    pub(crate) unsafe fn client_interface(name: &CStr) -> usize {
        unsafe {
            #[link(name = "dl")]
            extern "C" {
                fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
                fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
            }
            const RTLD_LAZY: c_int = 0x1;
            const RTLD_NOLOAD: c_int = 0x4;
            let Some((_, path)) = loaded_module(&[b"/client.so"]) else {
                return 0;
            };
            let handle = dlopen(path.as_ptr(), RTLD_LAZY | RTLD_NOLOAD);
            if handle.is_null() {
                return 0;
            }
            let create = dlsym(handle, c"CreateInterface".as_ptr());
            if create.is_null() {
                return 0;
            }
            let create: engine_fn!((*const c_char, *mut c_int) -> usize) =
                std::mem::transmute(create);
            create(name.as_ptr(), std::ptr::null_mut())
        }
    }

    /// True when this library was loaded as the client module (file name gmcl_*).
    #[cfg(feature = "client")]
    pub(crate) unsafe fn is_client_module() -> bool {
        unsafe {
            #[repr(C)]
            struct DlInfo {
                fname: *const c_char,
                fbase: *mut c_void,
                sname: *const c_char,
                saddr: *mut c_void,
            }
            #[link(name = "dl")]
            extern "C" {
                fn dladdr(addr: *const c_void, info: *mut DlInfo) -> c_int;
            }
            let mut info = DlInfo {
                fname: std::ptr::null(),
                fbase: std::ptr::null_mut(),
                sname: std::ptr::null(),
                saddr: std::ptr::null_mut(),
            };
            if dladdr(is_client_module as *const c_void, &mut info) == 0 || info.fname.is_null() {
                return false;
            }
            let path = CStr::from_ptr(info.fname).to_bytes();
            let name = path.rsplit(|&c| c == b'/').next().unwrap_or(path);
            name.starts_with(b"gmcl_")
        }
    }
}

#[cfg(target_os = "linux")]
pub(crate) use lin as os;
#[cfg(windows)]
pub(crate) use win as os;

#[cfg(windows)]
pub(crate) mod win {
    #[allow(clippy::wildcard_imports)]
    // the Windows code needs more names than the Linux server uses
    use super::*;

    #[link(name = "kernel32")]
    extern "system" {
        fn GetModuleHandleA(name: *const c_char) -> *mut c_void;
        #[cfg(feature = "server")]
        fn VirtualAlloc(addr: *mut c_void, size: usize, ty: u32, prot: u32) -> *mut c_void;
        fn VirtualProtect(addr: *mut c_void, size: usize, new: u32, old: *mut u32) -> i32;
        #[cfg(feature = "server")]
        fn FlushInstructionCache(process: *mut c_void, addr: *const c_void, size: usize) -> i32;
        #[cfg(feature = "server")]
        fn GetCurrentProcess() -> *mut c_void;
    }
    #[cfg(feature = "server")]
    const MEM_COMMIT_RESERVE: u32 = 0x3000;
    const PAGE_EXECUTE_READWRITE: u32 = 0x40;

    #[cfg(feature = "server")]
    unsafe fn alloc_exec(size: usize) -> *mut u8 {
        unsafe {
            VirtualAlloc(
                std::ptr::null_mut(),
                size,
                MEM_COMMIT_RESERVE,
                PAGE_EXECUTE_READWRITE,
            )
            .cast::<u8>()
        }
    }
    unsafe fn make_writable(p: *mut u8, n: usize) -> Option<u32> {
        unsafe {
            let mut old = 0u32;
            if VirtualProtect(p.cast::<c_void>(), n, PAGE_EXECUTE_READWRITE, &mut old) != 0 {
                Some(old)
            } else {
                None
            }
        }
    }
    unsafe fn restore_protect(p: *mut u8, n: usize, old: u32) {
        unsafe {
            let mut tmp = 0u32;
            VirtualProtect(p.cast::<c_void>(), n, old, &mut tmp);
        }
    }
    #[cfg(feature = "server")]
    unsafe fn flush(p: *mut u8, n: usize) {
        unsafe {
            FlushInstructionCache(GetCurrentProcess(), p as *const c_void, n);
        }
    }

    pub(crate) const PLATFORM: Platform = Platform {
        #[cfg(feature = "server")]
        alloc_exec,
        make_writable,
        restore_protect,
        #[cfg(feature = "server")]
        flush,
    };

    #[cfg(feature = "server")]
    pub(crate) unsafe fn server_base() -> *const u8 {
        unsafe { GetModuleHandleA(c"server.dll".as_ptr()) as *const u8 }
    }

    #[cfg(feature = "client")]
    pub(crate) unsafe fn client_base() -> *const u8 {
        unsafe { GetModuleHandleA(c"client.dll".as_ptr()) as *const u8 }
    }

    #[cfg(feature = "client")]
    /// An interface of client.dll (`CreateInterface("VClientEntityList003")`).
    pub(crate) unsafe fn client_interface(name: &CStr) -> usize {
        unsafe {
            #[link(name = "kernel32")]
            extern "system" {
                fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *mut c_void;
            }
            let module = client_base() as *mut c_void;
            if module.is_null() {
                return 0;
            }
            let create = GetProcAddress(module, c"CreateInterface".as_ptr());
            if create.is_null() {
                return 0;
            }
            let create: engine_fn!((*const c_char, *mut c_int) -> usize) =
                std::mem::transmute(create);
            create(name.as_ptr(), std::ptr::null_mut())
        }
    }
}
