// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67
//! `waterlevelercore` - per-entity water level overrides and fake water volumes for
//! Garry's Mod, as a server module (`gmsv_`) and a client module (`gmcl_`) built from this
//! one crate with the `server` and `client` features (see `Cargo.toml`). The volumes work on
//! the 64-bit Windows server and client and on the 64-bit Linux server and client.
//!
//! The server module registers the global table `waterlevelercore` (the `waterleveler` Lua
//! library in `addon/waterleveler/lua/includes/modules` wraps it and adds helpers) and methods
//! on the Entity metatable:
//!
//! ```lua
//! require("waterlevelercore")   -- raw module; the `waterleveler` Lua library wraps it
//! waterleveler.Set(ent, 3)        -- or ent:SetWaterLevelOverride(3)
//! waterleveler.Get(ent)           -- or ent:GetWaterLevelOverride()  -> number | nil
//! waterleveler.Clear(ent)         -- or ent:ClearWaterLevelOverride()
//! waterleveler.ClearAll()
//! waterleveler.IsHooked()         -- true if the engine hook is active
//! waterleveler.HookStatus()       -- human readable hook status
//! waterleveler.HookHits()         -- how many engine calls were overridden
//! waterleveler.SetSaveKey("m_nWaterLevel") -- datamap key written every tick; "" disables
//!
//! -- Water volumes (Windows x64 and Linux x64 server): world-space boxes the engine treats as water. The client
//! -- module has the volume functions, the counters, `SetDebug`, `TakeDebugLog` and `HookStatus`.
//! local id = waterleveler.AddVolume(x1, y1, z1, x2, y2, z2 [, "water" | "slime"])
//! waterleveler.RemoveVolume(id)
//! waterleveler.ClearVolumes()
//! waterleveler.VolumeCount()
//! waterleveler.PointInVolume(x, y, z)  -- pure lookup
//! waterleveler.IsContentsHooked()
//! waterleveler.ContentsHits()          -- engine point queries answered with water
//! waterleveler.TraceHits()             -- traces rewritten as water-surface hits
//! waterleveler.BubbleClips()           -- server: bullet bubble trails clipped to a volume
//! waterleveler.ExplosionFixes()        -- server: explosives forced into water
//! waterleveler.ExplosionsRedirected()  -- server: explosion events sent as the water explosion
//! waterleveler.AttachFluid(physobj, nx, ny, nz, dist [, damping] [, options]) -- server: buoyancy
//! waterleveler.DetachFluid(id)
//! waterleveler.FluidCount()
//! waterleveler.SetDebug(1)             -- log what the hooks answer near a volume
//! waterleveler.TakeDebugLog()          -- ...and read (and empty) that log
//! ```
//!
//! Water volumes work by swapping vtable slots of the engine's `IEngineTrace` object:
//! slot 0 (`GetPointContents`) so points inside a box also report `CONTENTS_WATER`, and slot 4
//! (`TraceRay`) so rays and hulls that enter a box are stopped at it like at a water brush
//! (bullet splashes, ripples, explosions). No machine code is patched for those two.
//!
//! The water level overrides have three layers:
//!   1. Engine hook (64-bit `server.dll` or Linux `server_srv.so` only): `CBaseEntity::SetWaterLevel`
//!      is detoured, so the engine itself uses the override. That also keeps
//!      networking and `GM:OnEntityWaterLevelChanged` consistent.
//!   2. `Entity:WaterLevel()` is wrapped, so Lua always sees the override.
//!   3. Every server tick the override is also written into the entity's
//!      datamap via `Entity:SetSaveValue(key, level)`. This covers code paths
//!      that write the byte directly instead of calling `SetWaterLevel`, and
//!      is the only engine-side layer on platforms without the hook.
//!
//! Overrides are dropped automatically when the entity is removed.
//!
//! The Lua C API is resolved at runtime from the already-loaded `lua_shared`
//! library (dlsym / `GetProcAddress`), so there are no link-time dependencies.

#![allow(clippy::missing_safety_doc)]
// The engine hooks are wired up on Windows x64 and the Linux x64 server; on other targets
// (32-bit) that code is compiled (and unit-tested) but not called.
#![cfg_attr(not(windows), allow(dead_code))]

#[cfg(not(any(feature = "server", feature = "client")))]
compile_error!("enable the `server` or the `client` feature");

#[cfg(feature = "server")]
use std::collections::BTreeMap;
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

type LuaState = *mut c_void;
type LuaFn = unsafe extern "C-unwind" fn(LuaState) -> c_int;

const LUA_GLOBALSINDEX: c_int = -10002;
#[cfg(feature = "server")]
const LUA_MULTRET: c_int = -1;
const LUA_TNONE: c_int = -1;
const LUA_TNIL: c_int = 0;
const LUA_TSTRING: c_int = 4;
const LUA_TNUMBER: c_int = 3;
#[cfg(feature = "server")]
const LUA_TTABLE: c_int = 5;
#[cfg(feature = "server")]
const LUA_TUSERDATA: c_int = 7;

#[cfg(feature = "server")]
const DEFAULT_SAVE_KEY: &str = "m_nWaterLevel";
#[cfg(feature = "server")]
const HOOK_ID: &CStr = c"gmsv_waterlevel";
#[cfg(feature = "server")]
const ORIG_KEY: &CStr = c"__waterlevel_orig";

// ---------------------------------------------------------------------------
// Lua C API (resolved at runtime)
// ---------------------------------------------------------------------------

struct Api {
    gettop: unsafe extern "C-unwind" fn(LuaState) -> c_int,
    settop: unsafe extern "C-unwind" fn(LuaState, c_int),
    #[cfg(feature = "server")]
    pushvalue: unsafe extern "C-unwind" fn(LuaState, c_int),
    pushnil: unsafe extern "C-unwind" fn(LuaState),
    pushnumber: unsafe extern "C-unwind" fn(LuaState, f64),
    pushboolean: unsafe extern "C-unwind" fn(LuaState, c_int),
    pushstring: unsafe extern "C-unwind" fn(LuaState, *const c_char) -> *const c_char,
    pushcclosure: unsafe extern "C-unwind" fn(LuaState, LuaFn, c_int),
    createtable: unsafe extern "C-unwind" fn(LuaState, c_int, c_int),
    #[cfg(feature = "server")]
    getfield: unsafe extern "C-unwind" fn(LuaState, c_int, *const c_char),
    setfield: unsafe extern "C-unwind" fn(LuaState, c_int, *const c_char),
    #[cfg(feature = "server")]
    call: unsafe extern "C-unwind" fn(LuaState, c_int, c_int),
    #[cfg(feature = "server")]
    pcall: unsafe extern "C-unwind" fn(LuaState, c_int, c_int, c_int) -> c_int,
    tonumber: unsafe extern "C-unwind" fn(LuaState, c_int) -> f64,
    #[cfg(feature = "server")]
    toboolean: unsafe extern "C-unwind" fn(LuaState, c_int) -> c_int,
    tolstring: unsafe extern "C-unwind" fn(LuaState, c_int, *mut usize) -> *const c_char,
    type_: unsafe extern "C-unwind" fn(LuaState, c_int) -> c_int,
    #[cfg(feature = "server")]
    touserdata: unsafe extern "C-unwind" fn(LuaState, c_int) -> *mut c_void,
}

static API: OnceLock<Api> = OnceLock::new();

#[inline]
fn api() -> &'static Api {
    // Only reachable after gmod13_open stored the table.
    API.get().expect("Lua API not loaded")
}

/// glibc's `dl_iterate_phdr` and the start of its `struct dl_phdr_info` (all that is read).
#[cfg(target_os = "linux")]
mod loaded_objects {
    use std::ffi::{c_char, c_int, c_void};

    #[repr(C)]
    pub(crate) struct DlPhdrInfo {
        pub(crate) addr: usize,
        pub(crate) name: *const c_char,
    }

    extern "C" {
        pub(crate) fn dl_iterate_phdr(
            callback: unsafe extern "C" fn(*const DlPhdrInfo, usize, *mut c_void) -> c_int,
            data: *mut c_void,
        ) -> c_int;
    }
}

#[cfg(target_os = "linux")]
unsafe fn find_symbol(name: &CStr) -> *mut c_void {
    unsafe {
        #[link(name = "dl")]
        extern "C" {
            fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
            fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
        }
        use loaded_objects::{dl_iterate_phdr, DlPhdrInfo};
        // lua_shared is already loaded by the game; RTLD_NOLOAD never loads anything new.
        const CANDIDATES: &[&CStr] = &[
            c"lua_shared.so",
            c"lua_shared_srv.so",
            c"bin/linux64/lua_shared.so",
            c"bin/linux64/lua_shared_srv.so",
            c"garrysmod/bin/linux64/lua_shared.so",
            c"garrysmod/bin/lua_shared_srv.so",
            c"garrysmod/bin/lua_shared.so",
            c"bin/linux32/lua_shared.so",
        ];
        const RTLD_LAZY: c_int = 0x1;
        const RTLD_NOLOAD: c_int = 0x4;

        // The game loads lua_shared by path, and dlopen only matches a bare name against a
        // loaded library's soname, so look the loaded library up by its full path first.
        unsafe extern "C" fn collect(
            info: *const DlPhdrInfo,
            _size: usize,
            data: *mut c_void,
        ) -> c_int {
            unsafe {
                let name = (*info).name;
                if !name.is_null() {
                    let name = CStr::from_ptr(name);
                    if name.to_bytes().windows(10).any(|w| w == b"lua_shared") {
                        (*data.cast::<Vec<CString>>()).push(name.to_owned());
                    }
                }
                0
            }
        }
        let mut loaded: Vec<CString> = Vec::new();
        dl_iterate_phdr(collect, (&raw mut loaded).cast::<c_void>());
        for path in &loaded {
            let handle = dlopen(path.as_ptr(), RTLD_LAZY | RTLD_NOLOAD);
            if !handle.is_null() {
                let sym = dlsym(handle, name.as_ptr());
                if !sym.is_null() {
                    return sym;
                }
            }
        }

        for lib in CANDIDATES {
            let handle = dlopen(lib.as_ptr(), RTLD_LAZY | RTLD_NOLOAD);
            if !handle.is_null() {
                let sym = dlsym(handle, name.as_ptr());
                if !sym.is_null() {
                    return sym;
                }
            }
        }
        // Fall back to the global scope (RTLD_DEFAULT == NULL on Linux).
        dlsym(std::ptr::null_mut(), name.as_ptr())
    }
}

#[cfg(windows)]
unsafe fn find_symbol(name: &CStr) -> *mut c_void {
    unsafe {
        #[link(name = "kernel32")]
        extern "system" {
            fn GetModuleHandleA(module_name: *const c_char) -> *mut c_void;
            fn GetProcAddress(module: *mut c_void, proc_name: *const c_char) -> *mut c_void;
        }
        let module = GetModuleHandleA(c"lua_shared.dll".as_ptr());
        if module.is_null() {
            return std::ptr::null_mut();
        }
        GetProcAddress(module, name.as_ptr())
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
compile_error!("waterlevelercore supports Linux and Windows only");

// The target of each transmute is the function pointer type of the `Api` field it fills.
#[allow(clippy::missing_transmute_annotations)]
unsafe fn load_api() -> Option<Api> {
    unsafe {
        macro_rules! load {
            ($name:literal) => {{
                let sym = find_symbol(CStr::from_bytes_with_nul_unchecked(
                    concat!($name, "\0").as_bytes(),
                ));
                if sym.is_null() {
                    eprintln!("[waterleveler] lua_shared has no symbol {}", $name);
                    return None;
                }
                std::mem::transmute::<*mut c_void, _>(sym)
            }};
        }
        Some(Api {
            gettop: load!("lua_gettop"),
            settop: load!("lua_settop"),
            #[cfg(feature = "server")]
            pushvalue: load!("lua_pushvalue"),
            pushnil: load!("lua_pushnil"),
            pushnumber: load!("lua_pushnumber"),
            pushboolean: load!("lua_pushboolean"),
            pushstring: load!("lua_pushstring"),
            pushcclosure: load!("lua_pushcclosure"),
            createtable: load!("lua_createtable"),
            #[cfg(feature = "server")]
            getfield: load!("lua_getfield"),
            setfield: load!("lua_setfield"),
            #[cfg(feature = "server")]
            call: load!("lua_call"),
            #[cfg(feature = "server")]
            pcall: load!("lua_pcall"),
            tonumber: load!("lua_tonumber"),
            #[cfg(feature = "server")]
            toboolean: load!("lua_toboolean"),
            tolstring: load!("lua_tolstring"),
            type_: load!("lua_type"),
            #[cfg(feature = "server")]
            touserdata: load!("lua_touserdata"),
        })
    }
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

#[cfg(feature = "server")]
/// Entity index -> overridden water level (0..=3).
static OVERRIDES: Mutex<BTreeMap<i32, u8>> = Mutex::new(BTreeMap::new());
#[cfg(feature = "server")]
/// `None` = default key, `Some("")` = engine writes disabled.
static SAVE_KEY: Mutex<Option<String>> = Mutex::new(None);

#[cfg(feature = "server")]
/// The settings of a fluid controller (`fluidparams_t`) besides its surface plane.
///
/// A map's water brush has torque, viscosity and aerodynamics at 0 (its fluid
/// data only carries the plane, current, damping and contents), so those are the
/// defaults; damping is 0.01.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FluidSettings {
    pub damping: f32,
    pub current: [f32; 3],
    pub torque: f32,
    pub viscosity: f32,
    pub aerodynamics: bool,
    /// `CONTENTS_WATER` or `CONTENTS_SLIME`.
    pub contents: c_int,
}

#[cfg(feature = "server")]
impl Default for FluidSettings {
    fn default() -> Self {
        Self {
            damping: 0.01,
            current: [0.0; 3],
            torque: 0.0,
            viscosity: 0.0,
            aerodynamics: false,
            contents: CONTENTS_WATER,
        }
    }
}

#[cfg(feature = "server")]
/// How many entries `OVERRIDES` has, kept up to date by `OverridesGuard`, so the
/// water-level hook (called for every entity) can skip the lock when there are none.
static OVERRIDE_COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[cfg(feature = "server")]
/// The overrides map, locked. When it is released the entry count is published.
struct OverridesGuard(std::sync::MutexGuard<'static, BTreeMap<i32, u8>>);

#[cfg(feature = "server")]
impl std::ops::Deref for OverridesGuard {
    type Target = BTreeMap<i32, u8>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[cfg(feature = "server")]
impl std::ops::DerefMut for OverridesGuard {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[cfg(feature = "server")]
impl Drop for OverridesGuard {
    fn drop(&mut self) {
        // Runs before the mutex is released, so no reader sees a count that is
        // older than the map it could then lock.
        OVERRIDE_COUNT.store(self.0.len(), std::sync::atomic::Ordering::Relaxed);
    }
}

#[cfg(feature = "server")]
fn overrides() -> OverridesGuard {
    OverridesGuard(
        OVERRIDES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    )
}

#[cfg(feature = "server")]
fn save_key() -> Option<CString> {
    let key = SAVE_KEY
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
        .unwrap_or_else(|| DEFAULT_SAVE_KEY.to_string());
    if key.is_empty() {
        None
    } else {
        CString::new(key).ok()
    }
}

// ---------------------------------------------------------------------------
// Water volumes (platform neutral store)
// ---------------------------------------------------------------------------
//
// Axis-aligned boxes in world space. With the engine hook installed, every
// point inside a box reports CONTENTS_WATER to the engine's own water logic.

const CONTENTS_WATER: c_int = 0x20;
const CONTENTS_SLIME: c_int = 0x10;
const MAX_VOLUMES: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Volume {
    id: u32,
    min: [f32; 3],
    max: [f32; 3],
    /// What the engine sees inside: `CONTENTS_WATER` or `CONTENTS_SLIME`.
    contents: c_int,
}

/// The volumes plus one box around all of them, so a query far from every volume is
/// rejected with a single comparison instead of a scan. Only the methods below change the
/// list, and each one recomputes the box.
#[derive(Clone, Debug, Default)]
struct VolumeList {
    list: Vec<Volume>,
    /// The smallest box containing every volume, or `None` while the list is empty.
    bounds: Option<([f32; 3], [f32; 3])>,
    /// The same boxes as one array per field, so the point test runs over plain slices
    /// the compiler can vectorize.
    lanes: Lanes,
    /// A tree over the boxes, for lists too long to scan.
    tree: Bvh,
}

/// Lists up to this long are scanned (the array copy); longer ones go through the tree.
const TREE_MIN_VOLUMES: usize = 16;
/// Volumes per leaf of the tree.
const TREE_LEAF: usize = 4;

/// One node of the tree: a box around everything below it. A leaf (`count > 0`) holds
/// `order[a..a + count]`; any other node has children `a` and `b`.
#[derive(Clone, Copy, Debug)]
struct TreeNode {
    lo: [f32; 3],
    hi: [f32; 3],
    a: u32,
    b: u32,
    count: u32,
}

/// A bounding-volume hierarchy over the volume boxes (median split on the longest axis),
/// so a point far from most volumes only visits the few boxes near it.
#[derive(Clone, Debug, Default)]
struct Bvh {
    nodes: Vec<TreeNode>,
    /// Volume indices, grouped so every leaf is a contiguous run.
    order: Vec<u32>,
}

impl Bvh {
    const fn new() -> Self {
        Self {
            nodes: Vec::new(),
            order: Vec::new(),
        }
    }

    fn build(vols: &[Volume]) -> Self {
        let mut tree = Self::new();
        if vols.len() > TREE_MIN_VOLUMES {
            tree.order = (0..vols.len() as u32).collect();
            tree.split(vols, 0, vols.len());
        }
        tree
    }

    /// Builds the node for `order[from..to]` and returns its index.
    fn split(&mut self, vols: &[Volume], from: usize, to: usize) -> u32 {
        let mut lo = [f32::INFINITY; 3];
        let mut hi = [f32::NEG_INFINITY; 3];
        let mut centre_lo = [f32::INFINITY; 3];
        let mut centre_hi = [f32::NEG_INFINITY; 3];
        for &i in &self.order[from..to] {
            let v = &vols[i as usize];
            for axis in 0..3 {
                lo[axis] = lo[axis].min(v.min[axis]);
                hi[axis] = hi[axis].max(v.max[axis]);
                let c = v.min[axis] + v.max[axis];
                centre_lo[axis] = centre_lo[axis].min(c);
                centre_hi[axis] = centre_hi[axis].max(c);
            }
        }
        let index = self.nodes.len() as u32;
        if to - from <= TREE_LEAF {
            self.nodes.push(TreeNode {
                lo,
                hi,
                a: from as u32,
                b: 0,
                count: (to - from) as u32,
            });
            return index;
        }
        self.nodes.push(TreeNode {
            lo,
            hi,
            a: 0,
            b: 0,
            count: 0,
        });
        let axis = (0..3)
            .max_by(|&x, &y| {
                (centre_hi[x] - centre_lo[x]).total_cmp(&(centre_hi[y] - centre_lo[y]))
            })
            .unwrap_or(0);
        self.order[from..to].sort_unstable_by(|&x, &y| {
            let cx = vols[x as usize].min[axis] + vols[x as usize].max[axis];
            let cy = vols[y as usize].min[axis] + vols[y as usize].max[axis];
            cx.total_cmp(&cy)
        });
        let mid = from + (to - from) / 2;
        let a = self.split(vols, from, mid);
        let b = self.split(vols, mid, to);
        let node = &mut self.nodes[index as usize];
        node.a = a;
        node.b = b;
        index
    }

    /// Calls `f` for every volume that contains `p`.
    fn visit(&self, vols: &[Volume], p: [f32; 3], mut f: impl FnMut(&Volume)) {
        // Median splits keep the tree shallow: the stack never holds more than depth + 1.
        let mut stack = [0u32; 64];
        let mut top = 1;
        if self.nodes.is_empty() {
            return;
        }
        while top > 0 {
            top -= 1;
            let node = &self.nodes[stack[top] as usize];
            if !(0..3).all(|i| p[i] >= node.lo[i] && p[i] <= node.hi[i]) {
                continue;
            }
            if node.count > 0 {
                for &i in &self.order[node.a as usize..(node.a + node.count) as usize] {
                    let v = &vols[i as usize];
                    if (0..3).all(|k| p[k] >= v.min[k] && p[k] <= v.max[k]) {
                        f(v);
                    }
                }
            } else {
                stack[top] = node.a;
                stack[top + 1] = node.b;
                top += 2;
            }
        }
    }
}

/// A struct-of-arrays copy of the volume list: entry `i` of every array is volume `i`.
#[derive(Clone, Debug, Default)]
struct Lanes {
    min: [Vec<f32>; 3],
    max: [Vec<f32>; 3],
    contents: Vec<c_int>,
}

impl VolumeList {
    const fn new() -> Self {
        Self {
            list: Vec::new(),
            bounds: None,
            lanes: Lanes {
                min: [Vec::new(), Vec::new(), Vec::new()],
                max: [Vec::new(), Vec::new(), Vec::new()],
                contents: Vec::new(),
            },
            tree: Bvh::new(),
        }
    }

    fn refresh_bounds(&mut self) {
        for axis in 0..3 {
            self.lanes.min[axis] = self.list.iter().map(|v| v.min[axis]).collect();
            self.lanes.max[axis] = self.list.iter().map(|v| v.max[axis]).collect();
        }
        self.lanes.contents = self.list.iter().map(|v| v.contents).collect();
        self.tree = Bvh::build(&self.list);
        self.bounds = self.list.iter().fold(None, |acc, v| {
            Some(acc.map_or((v.min, v.max), |(lo, hi)| {
                (
                    [
                        lo[0].min(v.min[0]),
                        lo[1].min(v.min[1]),
                        lo[2].min(v.min[2]),
                    ],
                    [
                        hi[0].max(v.max[0]),
                        hi[1].max(v.max[1]),
                        hi[2].max(v.max[2]),
                    ],
                )
            }))
        });
    }

    fn push(&mut self, volume: Volume) {
        self.list.push(volume);
        self.refresh_bounds();
    }

    fn clear(&mut self) {
        self.list.clear();
        self.refresh_bounds();
    }

    fn retain(&mut self, keep: impl FnMut(&Volume) -> bool) {
        self.list.retain(keep);
        self.refresh_bounds();
    }

    /// Replaces every volume.
    #[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
    fn set(&mut self, volumes: Vec<Volume>) {
        self.list = volumes;
        self.refresh_bounds();
    }

    /// The contents of every volume containing `p`, or'ed together (0 if none). Same result
    /// as `contents_at` on the plain list, computed without a branch per volume.
    #[allow(clippy::needless_bitwise_bool)] // `&` on purpose: no early exit, so the loop vectorizes
    fn contents_at(&self, p: [f32; 3]) -> c_int {
        if self.list.len() > TREE_MIN_VOLUMES {
            let mut contents = 0;
            self.tree.visit(&self.list, p, |v| contents |= v.contents);
            return contents;
        }
        let l = &self.lanes;
        let n = l.contents.len();
        let (x0, y0, z0) = (&l.min[0][..n], &l.min[1][..n], &l.min[2][..n]);
        let (x1, y1, z1) = (&l.max[0][..n], &l.max[1][..n], &l.max[2][..n]);
        let mut contents = 0;
        for i in 0..n {
            // `&`, not `&&`: no early exit, so the loop is a straight line of compares.
            let inside = (p[0] >= x0[i])
                & (p[0] <= x1[i])
                & (p[1] >= y0[i])
                & (p[1] <= y1[i])
                & (p[2] >= z0[i])
                & (p[2] <= z1[i]);
            contents |= l.contents[i] & -c_int::from(inside);
        }
        contents
    }

    /// True if `p` is inside any volume.
    #[allow(clippy::needless_bitwise_bool)] // `&` on purpose: no early exit, so the loop vectorizes
    fn contains(&self, p: [f32; 3]) -> bool {
        if self.list.len() > TREE_MIN_VOLUMES {
            let mut inside = false;
            self.tree.visit(&self.list, p, |_| inside = true);
            return inside;
        }
        let l = &self.lanes;
        let n = l.contents.len();
        let (x0, y0, z0) = (&l.min[0][..n], &l.min[1][..n], &l.min[2][..n]);
        let (x1, y1, z1) = (&l.max[0][..n], &l.max[1][..n], &l.max[2][..n]);
        let mut inside = false;
        for i in 0..n {
            inside |= (p[0] >= x0[i])
                & (p[0] <= x1[i])
                & (p[1] >= y0[i])
                & (p[1] <= y1[i])
                & (p[2] >= z0[i])
                & (p[2] <= z1[i]);
        }
        inside
    }

    /// Top (surface height) of the highest water volume containing `p`.
    #[cfg(feature = "server")]
    fn surface_z(&self, p: [f32; 3]) -> Option<f32> {
        let mut top: Option<f32> = None;
        let mut take = |v: &Volume| {
            if v.contents & CONTENTS_WATER != 0 {
                top = Some(top.map_or(v.max[2], |t| t.max(v.max[2])));
            }
        };
        if self.list.len() > TREE_MIN_VOLUMES {
            self.tree.visit(&self.list, p, take);
        } else {
            for v in &self.list {
                if (0..3).all(|i| p[i] >= v.min[i] && p[i] <= v.max[i]) {
                    take(v);
                }
            }
        }
        top
    }

    /// False if `p` is certainly in no volume (true means "maybe": scan the list).
    fn may_contain(&self, p: [f32; 3]) -> bool {
        self.bounds
            .is_some_and(|(lo, hi)| (0..3).all(|i| p[i] >= lo[i] && p[i] <= hi[i]))
    }

    /// True if `p` is inside the box around every volume grown by `margin` (the debug log
    /// uses it to catch queries that land just outside a volume).
    fn near(&self, p: [f32; 3], margin: f32) -> bool {
        self.bounds.is_some_and(|(lo, hi)| {
            (0..3).all(|i| p[i] >= lo[i] - margin && p[i] <= hi[i] + margin)
        })
    }
}

impl std::ops::Deref for VolumeList {
    type Target = [Volume];

    fn deref(&self) -> &[Volume] {
        &self.list
    }
}

/// The writers' copy of the volumes. Edits go through `volumes()`, which publishes the
/// result to `SNAPSHOT` when the guard is released.
static VOLUMES: Mutex<VolumeList> = Mutex::new(VolumeList::new());
/// What the engine hooks read: an immutable copy of the list, replaced as a whole, so a
/// hook never waits for (or skips because of) an edit in progress. `None` while empty.
static SNAPSHOT: RwLock<Option<Arc<VolumeList>>> = RwLock::new(None);
static NEXT_VOLUME_ID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static VOLUME_HITS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The volumes, locked for reading or editing. If the list was changed, the new copy is
/// published for the hooks when this is released.
struct VolumesGuard {
    guard: std::sync::MutexGuard<'static, VolumeList>,
    changed: bool,
}

impl std::ops::Deref for VolumesGuard {
    type Target = VolumeList;

    fn deref(&self) -> &VolumeList {
        &self.guard
    }
}

impl std::ops::DerefMut for VolumesGuard {
    fn deref_mut(&mut self) -> &mut VolumeList {
        self.changed = true;
        &mut self.guard
    }
}

impl Drop for VolumesGuard {
    fn drop(&mut self) {
        if self.changed {
            // Runs before the mutex is released, so copies are published in edit order.
            let copy = (!self.guard.is_empty()).then(|| Arc::new((*self.guard).clone()));
            *SNAPSHOT
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = copy;
        }
    }
}

fn volumes() -> VolumesGuard {
    VolumesGuard {
        guard: VOLUMES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
        changed: false,
    }
}

/// The current volumes for an engine hook, or `None` when there are none. Never waits for an
/// edit; the lock is only held for the instant it takes to swap the copy.
#[cfg(all(test, target_os = "linux"))]
fn snapshot() -> Option<Arc<VolumeList>> {
    match SNAPSHOT.try_read() {
        Ok(copy) => copy.clone(),
        Err(std::sync::TryLockError::Poisoned(e)) => e.into_inner().clone(),
        Err(std::sync::TryLockError::WouldBlock) => None,
    }
}

/// Like `snapshot`, but the copy is only taken (a pointer clone, two atomic operations) when
/// `maybe` says the query could touch a volume, which is rare: `maybe` runs on the shared
/// copy, so the common case costs one read lock and a comparison.
fn snapshot_if(maybe: impl FnOnce(&VolumeList) -> bool) -> Option<Arc<VolumeList>> {
    match SNAPSHOT.try_read() {
        Ok(copy) => copy.as_ref().filter(|v| maybe(v)).cloned(),
        Err(std::sync::TryLockError::Poisoned(e)) => {
            e.into_inner().as_ref().filter(|v| maybe(v)).cloned()
        }
        Err(std::sync::TryLockError::WouldBlock) => None,
    }
}

/// Counts one event. A plain load and store, not an atomic add: these diagnostic counters
/// sit in hot hooks, and a count lost to a race between threads does not matter.
fn bump(counter: &std::sync::atomic::AtomicU64) {
    use std::sync::atomic::Ordering::Relaxed;
    counter.store(counter.load(Relaxed).wrapping_add(1), Relaxed);
}

/// Top (surface height) of the highest water volume containing `p` (slime is
/// not water: the engine's water explosion needs `CONTENTS_WATER`).
#[cfg(all(test, target_os = "linux"))]
fn surface_z_at(vols: &[Volume], p: [f32; 3]) -> Option<f32> {
    vols.iter()
        .filter(|v| {
            v.contents & CONTENTS_WATER != 0 && (0..3).all(|i| p[i] >= v.min[i] && p[i] <= v.max[i])
        })
        .map(|v| v.max[2])
        .fold(None, |a, z| Some(a.map_or(z, |a: f32| a.max(z))))
}

/// The contents of every volume containing `p`, or'ed together (0 if none). The reference
/// the tests hold `VolumeList::contents_at` to.
#[cfg(all(test, target_os = "linux"))]
fn contents_at(vols: &[Volume], p: [f32; 3]) -> c_int {
    vols.iter()
        .filter(|v| (0..3).all(|i| p[i] >= v.min[i] && p[i] <= v.max[i]))
        .fold(0, |c, v| c | v.contents)
}

#[cfg(all(test, target_os = "linux"))]
fn point_in_volumes(vols: &[Volume], p: [f32; 3]) -> bool {
    vols.iter()
        .any(|v| (0..3).all(|i| p[i] >= v.min[i] && p[i] <= v.max[i]))
}

// ---------------------------------------------------------------------------
// Engine hook: CBaseEntity::SetWaterLevel(int) (x86-64 only)
// ---------------------------------------------------------------------------
//
// Layout facts below were read out of the 64-bit Windows server.dll:
//   * SetWaterLevel(this = rcx, level = edx) is a non-virtual function with 18
//     direct call sites; its first 15 bytes are three position-independent
//     `mov [rsp+x], reg` instructions, so they can be relocated verbatim.
//   * m_nWaterLevel is a byte at this+0x2ab.
//   * The entity index is `*(i16*)(*(void**)(this + 0x30) + 6)` (m_pPev ->
//     m_EdictIndex); this is exactly what the Lua binding Entity:EntIndex does.

#[cfg(target_arch = "x86_64")]
mod engine_hook;

// ---------------------------------------------------------------------------
// Client realm (gmcl_waterlevelercore)
// ---------------------------------------------------------------------------
//
// Nothing about the water explosion is patched on the client: its effect finds the
// surface itself with a MASK_WATER trace up from the blast point and picks the
// underwater sound from that distance, so the client's hooked trace (see `engine_hook`)
// is all it needs.

#[cfg(all(
    any(windows, target_os = "linux"),
    target_arch = "x86_64",
    feature = "server",
    feature = "client"
))]
mod client_module;

#[cfg(all(target_arch = "x86_64", any(windows, target_os = "linux")))]
mod hook_state;

#[cfg(not(all(target_arch = "x86_64", any(windows, target_os = "linux"))))]
mod hook_state {
    #[cfg(feature = "server")]
    pub(crate) const unsafe fn install() -> Result<usize, &'static str> {
        Err("engine hook is only available for the 64-bit Windows and Linux servers")
    }
    #[cfg(feature = "server")]
    pub(crate) const unsafe fn uninstall() {}
    #[cfg(feature = "server")]
    pub(crate) const fn active() -> bool {
        false
    }
    #[cfg(feature = "server")]
    pub(crate) const unsafe fn install_contents() -> Result<String, &'static str> {
        Err("engine hook is only available for the 64-bit Windows and Linux servers")
    }
    pub(crate) const unsafe fn uninstall_contents() {}
    pub(crate) const fn contents_active() -> bool {
        false
    }
    #[cfg(feature = "server")]
    pub(crate) const unsafe fn fluid_attach(
        _: usize,
        _: [f32; 4],
        _: &super::FluidSettings,
    ) -> Result<u32, &'static str> {
        Err("fluids are only available for the 64-bit Windows and Linux servers")
    }
    #[cfg(feature = "server")]
    pub(crate) const unsafe fn fluid_detach(_: u32) -> bool {
        false
    }
    #[cfg(feature = "server")]
    pub(crate) const unsafe fn fluid_detach_all() {}
    #[cfg(feature = "server")]
    pub(crate) const fn fluid_count() -> usize {
        0
    }
}

static HOOK_STATUS: Mutex<String> = Mutex::new(String::new());

#[cfg(feature = "server")]
fn hook_hits() -> u64 {
    #[cfg(target_arch = "x86_64")]
    {
        engine_hook::HITS.load(std::sync::atomic::Ordering::Relaxed)
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        0
    }
}

// ---------------------------------------------------------------------------
// Lua helpers
// ---------------------------------------------------------------------------

#[cfg(feature = "server")]
/// Returns the entity index of the Entity userdata at absolute stack index `idx`.
/// Rejects non-userdata, NULL entities and the world.
unsafe fn ent_index(l: LuaState, idx: c_int) -> Option<i32> {
    unsafe {
        let a = api();
        if (a.type_)(l, idx) != LUA_TUSERDATA {
            return None;
        }
        let top = (a.gettop)(l);
        (a.getfield)(l, idx, c"EntIndex".as_ptr());
        (a.pushvalue)(l, idx);
        let ok = (a.pcall)(l, 1, 1, 0) == 0;
        let n = if ok && (a.type_)(l, -1) == LUA_TNUMBER {
            Some((a.tonumber)(l, -1) as i32)
        } else {
            None
        };
        (a.settop)(l, top);
        n.filter(|&i| i > 0)
    }
}

unsafe fn fail(l: LuaState, msg: &CStr) -> c_int {
    unsafe {
        let a = api();
        (a.pushnil)(l);
        (a.pushstring)(l, msg.as_ptr());
        2
    }
}

#[cfg(feature = "server")]
/// The value of the `developer` convar (0 when it cannot be read).
unsafe fn developer_level(l: LuaState) -> f64 {
    unsafe {
        let a = api();
        let base = (a.gettop)(l);
        let mut level = 0.0;
        (a.getfield)(l, LUA_GLOBALSINDEX, c"GetConVar".as_ptr());
        (a.pushstring)(l, c"developer".as_ptr());
        // The convar object is a userdata; its GetInt is looked up on its metatable.
        if (a.pcall)(l, 1, 1, 0) == 0 && (a.type_)(l, -1) == LUA_TUSERDATA {
            (a.getfield)(l, -1, c"GetInt".as_ptr());
            (a.pushvalue)(l, -2);
            if (a.pcall)(l, 1, 1, 0) == 0 {
                level = (a.tonumber)(l, -1);
            }
        }

        (a.settop)(l, base);
        level
    }
}

#[cfg(feature = "server")]
unsafe fn lua_print(l: LuaState, msg: &str) {
    unsafe {
        let a = api();
        let Ok(text) = CString::new(msg) else { return };
        let base = (a.gettop)(l);
        (a.getfield)(l, LUA_GLOBALSINDEX, c"print".as_ptr());
        (a.pushstring)(l, text.as_ptr());
        let _ = (a.pcall)(l, 1, 0, 0);
        (a.settop)(l, base);
    }
}

#[cfg(feature = "server")]
/// Looks the entity up by index and, if still valid, writes the level into its
/// datamap (when `key` is set). Returns false if the entity no longer exists.
unsafe fn apply_level(l: LuaState, idx: i32, level: u8, key: Option<&CStr>) -> bool {
    unsafe {
        let a = api();
        let base = (a.gettop)(l);

        (a.getfield)(l, LUA_GLOBALSINDEX, c"Entity".as_ptr());
        (a.pushnumber)(l, f64::from(idx));
        if (a.pcall)(l, 1, 1, 0) != 0 {
            (a.settop)(l, base);
            return false;
        }
        let ent = base + 1;

        (a.getfield)(l, ent, c"IsValid".as_ptr());
        (a.pushvalue)(l, ent);
        let valid = (a.pcall)(l, 1, 1, 0) == 0 && (a.toboolean)(l, -1) != 0;
        (a.settop)(l, ent);
        if !valid {
            (a.settop)(l, base);
            return false;
        }

        if let Some(key) = key {
            (a.getfield)(l, ent, c"SetSaveValue".as_ptr());
            (a.pushvalue)(l, ent);
            (a.pushstring)(l, key.as_ptr());
            (a.pushnumber)(l, f64::from(level));
            let _ = (a.pcall)(l, 3, 0, 0); // best effort
        }
        (a.settop)(l, base);
        true
    }
}

// ---------------------------------------------------------------------------
// Lua-callable functions
// ---------------------------------------------------------------------------

#[cfg(feature = "server")]
/// waterleveler.Set(ent, level) / ent:SetWaterLevelOverride(level)
unsafe extern "C-unwind" fn l_set(l: LuaState) -> c_int {
    unsafe {
        let a = api();
        let Some(idx) = ent_index(l, 1) else {
            return fail(l, c"invalid entity");
        };
        if (a.type_)(l, 2) != LUA_TNUMBER {
            return fail(l, c"level must be a number");
        }
        let level = (a.tonumber)(l, 2);
        if !level.is_finite() || level.fract() != 0.0 || !(0.0..=3.0).contains(&level) {
            return fail(l, c"level must be an integer from 0 to 3");
        }
        let level = level as u8;
        overrides().insert(idx, level);
        apply_level(l, idx, level, save_key().as_deref());
        (a.pushboolean)(l, 1);
        1
    }
}

#[cfg(feature = "server")]
/// waterleveler.Get(ent) / `ent:GetWaterLevelOverride()`
unsafe extern "C-unwind" fn l_get(l: LuaState) -> c_int {
    unsafe {
        let a = api();
        let level = ent_index(l, 1).and_then(|idx| overrides().get(&idx).copied());
        match level {
            Some(v) => (a.pushnumber)(l, f64::from(v)),
            None => (a.pushnil)(l),
        }
        1
    }
}

#[cfg(feature = "server")]
/// waterleveler.Clear(ent) / `ent:ClearWaterLevelOverride()`
unsafe extern "C-unwind" fn l_clear(l: LuaState) -> c_int {
    unsafe {
        let a = api();
        let removed = ent_index(l, 1).is_some_and(|idx| overrides().remove(&idx).is_some());
        (a.pushboolean)(l, c_int::from(removed));
        1
    }
}

#[cfg(feature = "server")]
/// `waterleveler.ClearAll()`
unsafe extern "C-unwind" fn l_clear_all(_l: LuaState) -> c_int {
    overrides().clear();
    0
}

#[cfg(feature = "server")]
/// waterleveler.SetSaveKey(key) - "" disables the per-tick datamap write.
unsafe extern "C-unwind" fn l_set_save_key(l: LuaState) -> c_int {
    unsafe {
        let a = api();
        let p = (a.tolstring)(l, 1, std::ptr::null_mut());
        if p.is_null() {
            return fail(l, c"key must be a string");
        }
        let key = CStr::from_ptr(p).to_string_lossy().into_owned();
        *SAVE_KEY
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(key);
        (a.pushboolean)(l, 1);
        1
    }
}

#[cfg(feature = "server")]
/// `waterleveler.IsHooked()`
unsafe extern "C-unwind" fn l_is_hooked(l: LuaState) -> c_int {
    unsafe {
        (api().pushboolean)(l, c_int::from(hook_state::active()));
        1
    }
}

/// `waterleveler.HookStatus()`
unsafe extern "C-unwind" fn l_hook_status(l: LuaState) -> c_int {
    unsafe {
        let status = HOOK_STATUS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        // The short commit hash: shows which build is really loaded.
        let status = format!("{status} [build {}]", env!("WL_BUILD"));
        let text = CString::new(status).unwrap_or_default();
        (api().pushstring)(l, text.as_ptr());
        1
    }
}

#[cfg(feature = "server")]
/// `waterleveler.HookHits()`
unsafe extern "C-unwind" fn l_hook_hits(l: LuaState) -> c_int {
    unsafe {
        (api().pushnumber)(l, hook_hits() as f64);
        1
    }
}

/// The optional liquid argument at stack index `idx`: "water" (default, also nil or
/// none) or "slime".
unsafe fn parse_liquid(l: LuaState, idx: c_int) -> Result<c_int, &'static CStr> {
    unsafe {
        let a = api();
        match (a.type_)(l, idx) {
            LUA_TNONE | LUA_TNIL => Ok(CONTENTS_WATER),
            LUA_TSTRING => {
                let p = (a.tolstring)(l, idx, std::ptr::null_mut());
                match CStr::from_ptr(p).to_bytes().to_ascii_lowercase().as_slice() {
                    b"water" => Ok(CONTENTS_WATER),
                    b"slime" => Ok(CONTENTS_SLIME),
                    _ => Err(c"liquid must be \"water\" or \"slime\""),
                }
            }
            _ => Err(c"liquid must be \"water\" or \"slime\""),
        }
    }
}

/// waterleveler.AddVolume(x1, y1, z1, x2, y2, z2 [, liquid]) -> id | nil, err
/// Adds a world-space box in which the engine sees water, or slime when `liquid` is
/// "slime" ("water" is the default).
unsafe extern "C-unwind" fn l_add_volume(l: LuaState) -> c_int {
    unsafe {
        let a = api();
        let mut v = [0f64; 6];
        for (i, slot) in v.iter_mut().enumerate() {
            let idx = i as c_int + 1;
            if (a.type_)(l, idx) != LUA_TNUMBER {
                return fail(l, c"AddVolume needs 6 numbers: x1, y1, z1, x2, y2, z2");
            }
            *slot = (a.tonumber)(l, idx);
            if !slot.is_finite() {
                return fail(l, c"AddVolume coordinates must be finite");
            }
        }
        let contents = match parse_liquid(l, 7) {
            Ok(c) => c,
            Err(msg) => return fail(l, msg),
        };
        let min = [
            v[0].min(v[3]) as f32,
            v[1].min(v[4]) as f32,
            v[2].min(v[5]) as f32,
        ];
        let max = [
            v[0].max(v[3]) as f32,
            v[1].max(v[4]) as f32,
            v[2].max(v[5]) as f32,
        ];
        let id = {
            let mut vols = volumes();
            let id = if vols.len() >= MAX_VOLUMES {
                None
            } else {
                let id = NEXT_VOLUME_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                vols.push(Volume {
                    id,
                    min,
                    max,
                    contents,
                });
                Some(id)
            };
            drop(vols);
            id
        };
        id.map_or_else(
            || fail(l, c"too many volumes (limit 256)"),
            |id| {
                (a.pushnumber)(l, f64::from(id));
                1
            },
        )
    }
}

/// waterleveler.RemoveVolume(id) -> bool
unsafe extern "C-unwind" fn l_remove_volume(l: LuaState) -> c_int {
    unsafe {
        let a = api();
        let id = (a.tonumber)(l, 1) as u32;
        let removed = {
            let mut vols = volumes();
            let before = vols.len();
            vols.retain(|v| v.id != id);
            vols.len() != before
        };
        (a.pushboolean)(l, c_int::from(removed));
        1
    }
}

/// `waterleveler.ClearVolumes()`
unsafe extern "C-unwind" fn l_clear_volumes(_l: LuaState) -> c_int {
    volumes().clear();
    0
}

/// `waterleveler.VolumeCount()`
unsafe extern "C-unwind" fn l_volume_count(l: LuaState) -> c_int {
    unsafe {
        let n = volumes().len();
        (api().pushnumber)(l, n as f64);
        1
    }
}

/// waterleveler.PointInVolume(x, y, z) -> bool (pure lookup, no engine needed)
unsafe extern "C-unwind" fn l_point_in_volume(l: LuaState) -> c_int {
    unsafe {
        let a = api();
        let p = [
            (a.tonumber)(l, 1) as f32,
            (a.tonumber)(l, 2) as f32,
            (a.tonumber)(l, 3) as f32,
        ];
        let inside = volumes().contains(p);
        (a.pushboolean)(l, c_int::from(inside));
        1
    }
}

/// `waterleveler.IsContentsHooked()`
unsafe extern "C-unwind" fn l_is_contents_hooked(l: LuaState) -> c_int {
    unsafe {
        (api().pushboolean)(l, c_int::from(hook_state::contents_active()));
        1
    }
}

/// `waterleveler.ContentsHits()` - engine point queries answered with water
unsafe extern "C-unwind" fn l_contents_hits(l: LuaState) -> c_int {
    unsafe {
        let n = VOLUME_HITS.load(std::sync::atomic::Ordering::Relaxed);
        (api().pushnumber)(l, n as f64);
        1
    }
}

/// `waterleveler.SetDebug(1|0)` - log what the hooks answer near a volume (read it with `TakeDebugLog`)
unsafe extern "C-unwind" fn l_set_debug(l: LuaState) -> c_int {
    unsafe {
        #[cfg(target_arch = "x86_64")]
        engine_hook::DEBUG.store(
            (api().tonumber)(l, 1) != 0.0,
            std::sync::atomic::Ordering::Relaxed,
        );
        #[cfg(not(target_arch = "x86_64"))]
        let _ = l;
        0
    }
}

/// `waterleveler.TakeDebugLog()` -> string: the log lines since the last call, empties the log
unsafe extern "C-unwind" fn l_take_debug_log(l: LuaState) -> c_int {
    unsafe {
        #[cfg(target_arch = "x86_64")]
        let text = engine_hook::take_debug_log();
        #[cfg(not(target_arch = "x86_64"))]
        let text = String::new();
        let text = CString::new(text.replace('\0', "")).unwrap_or_default();
        (api().pushstring)(l, text.as_ptr());
        1
    }
}

/// `waterleveler.TraceHits()` - engine traces rewritten as water-surface hits
unsafe extern "C-unwind" fn l_trace_hits(l: LuaState) -> c_int {
    unsafe {
        #[cfg(target_arch = "x86_64")]
        let n = engine_hook::TRACE_HITS.load(std::sync::atomic::Ordering::Relaxed);
        #[cfg(not(target_arch = "x86_64"))]
        let n = 0u64;
        (api().pushnumber)(l, n as f64);
        1
    }
}

#[cfg(feature = "server")]
/// waterleveler.AttachFluid(physobj, nx, ny, nz, dist [, damping] [, options]) -> id | nil, err
/// Makes `physobj` an engine fluid (buoyancy + drag for props inside it).
/// The plane is in world space with the normal pointing out of the fluid;
/// for a water volume that is (0, 0, 1) and the world Z of its top face.
/// `options` is an optional table (it may also be the 6th argument): damping,
/// torque, viscosity, aerodynamics (boolean), contents ("water"/"slime"), currentx, currenty, currentz.
/// Defaults are those of a map's water brush (see `FluidSettings`).
unsafe extern "C-unwind" fn l_attach_fluid(l: LuaState) -> c_int {
    unsafe {
        let a = api();
        if (a.type_)(l, 1) != LUA_TUSERDATA {
            return fail(l, c"AttachFluid needs a PhysObj as its first argument");
        }
        // GMod userdata block: { void *data; u8 type }, PhysObj is type 12.
        let ud = (a.touserdata)(l, 1) as *const *mut c_void;
        if ud.is_null() || *ud.add(1).cast::<u8>() != 12 {
            return fail(l, c"first argument is not a PhysObj");
        }
        let obj = ud.read();
        if obj.is_null() {
            return fail(l, c"PhysObj is invalid");
        }
        let mut v = [0f32; 4];
        for (i, slot) in v.iter_mut().enumerate() {
            let idx = i as c_int + 2;
            if (a.type_)(l, idx) != LUA_TNUMBER {
                return fail(
                    l,
                    c"AttachFluid needs 4 numbers after the PhysObj: nx, ny, nz, dist",
                );
            }
            let n = (a.tonumber)(l, idx);
            if !n.is_finite() {
                return fail(l, c"plane values must be finite");
            }
            *slot = n as f32;
        }
        let mut fluid = FluidSettings::default();
        let mut opts = 0;
        if (a.type_)(l, 6) == LUA_TNUMBER {
            fluid.damping = (a.tonumber)(l, 6) as f32;
        } else if (a.type_)(l, 6) == LUA_TTABLE {
            opts = 6;
        }
        if (a.type_)(l, 7) == LUA_TTABLE {
            opts = 7;
        }
        if opts != 0 {
            let num = |name: &CStr, slot: &mut f32| -> bool {
                (a.getfield)(l, opts, name.as_ptr());
                let present = (a.type_)(l, -1) == LUA_TNUMBER;
                if present {
                    *slot = (a.tonumber)(l, -1) as f32;
                }
                (a.settop)(l, -2);
                present
            };
            num(c"damping", &mut fluid.damping);
            num(c"torque", &mut fluid.torque);
            num(c"viscosity", &mut fluid.viscosity);
            num(c"currentx", &mut fluid.current[0]);
            num(c"currenty", &mut fluid.current[1]);
            num(c"currentz", &mut fluid.current[2]);
            (a.getfield)(l, opts, c"aerodynamics".as_ptr());
            fluid.aerodynamics = (a.toboolean)(l, -1) != 0;
            (a.settop)(l, -2);
            (a.getfield)(l, opts, c"contents".as_ptr());
            let liquid = parse_liquid(l, -1);
            (a.settop)(l, -2);
            match liquid {
                Ok(c) => fluid.contents = c,
                Err(msg) => return fail(l, msg),
            }
        }
        let finite = fluid.damping.is_finite()
            && fluid.torque.is_finite()
            && fluid.viscosity.is_finite()
            && fluid.current.iter().all(|c| c.is_finite());
        if !finite {
            return fail(l, c"fluid settings must be finite numbers");
        }
        match hook_state::fluid_attach(obj as usize, v, &fluid) {
            Ok(id) => {
                (a.pushnumber)(l, f64::from(id));
                1
            }
            Err(why) => {
                let msg = CString::new(why).unwrap_or_default();
                fail(l, &msg)
            }
        }
    }
}

#[cfg(feature = "server")]
/// waterleveler.DetachFluid(id) -> bool. Call before removing the entity.
unsafe extern "C-unwind" fn l_detach_fluid(l: LuaState) -> c_int {
    unsafe {
        let a = api();
        let ok =
            (a.type_)(l, 1) == LUA_TNUMBER && hook_state::fluid_detach((a.tonumber)(l, 1) as u32);
        (a.pushboolean)(l, c_int::from(ok));
        1
    }
}

#[cfg(feature = "server")]
/// `waterleveler.FluidCount()`
unsafe extern "C-unwind" fn l_fluid_count(l: LuaState) -> c_int {
    unsafe {
        (api().pushnumber)(l, hook_state::fluid_count() as f64);
        1
    }
}

#[cfg(feature = "server")]
/// `waterleveler.BubbleClips()` - bullet bubble trails clipped to a volume
unsafe extern "C-unwind" fn l_bubble_clips(l: LuaState) -> c_int {
    unsafe {
        #[cfg(target_arch = "x86_64")]
        let n = engine_hook::BUBBLE_CLIPS.load(std::sync::atomic::Ordering::Relaxed);
        #[cfg(not(target_arch = "x86_64"))]
        let n = 0u64;
        (api().pushnumber)(l, n as f64);
        1
    }
}

#[cfg(feature = "server")]
/// `waterleveler.ExplosionFixes()` - explosives whose water level was raised
unsafe extern "C-unwind" fn l_explosion_fixes(l: LuaState) -> c_int {
    unsafe {
        #[cfg(target_arch = "x86_64")]
        let n = engine_hook::EXPLOSION_FIXES.load(std::sync::atomic::Ordering::Relaxed);
        #[cfg(not(target_arch = "x86_64"))]
        let n = 0u64;
        (api().pushnumber)(l, n as f64);
        1
    }
}

#[cfg(feature = "server")]
/// `waterleveler.ExplosionsRedirected()` - explosion events sent as the water explosion
unsafe extern "C-unwind" fn l_explosions_redirected(l: LuaState) -> c_int {
    unsafe {
        #[cfg(target_arch = "x86_64")]
        let n = engine_hook::EXPLOSIONS_REDIRECTED.load(std::sync::atomic::Ordering::Relaxed);
        #[cfg(not(target_arch = "x86_64"))]
        let n = 0u64;
        (api().pushnumber)(l, n as f64);
        1
    }
}

#[cfg(feature = "server")]
/// Replacement for `Entity:WaterLevel()`. Upvalue 1 is the original function.
unsafe extern "C-unwind" fn l_wrapped_water_level(l: LuaState) -> c_int {
    unsafe {
        let a = api();
        if let Some(idx) = ent_index(l, 1) {
            let level = overrides().get(&idx).copied(); // guard dropped here
            if let Some(level) = level {
                (a.pushnumber)(l, f64::from(level));
                return 1;
            }
        }
        let nargs = (a.gettop)(l);
        (a.pushvalue)(l, LUA_GLOBALSINDEX - 1); // upvalue 1
        for i in 1..=nargs {
            (a.pushvalue)(l, i);
        }
        (a.call)(l, nargs, LUA_MULTRET);
        (a.gettop)(l) - nargs
    }
}

#[cfg(feature = "server")]
/// Hook "Tick": re-apply every override, drop entities that are gone.
unsafe extern "C-unwind" fn l_tick(l: LuaState) -> c_int {
    unsafe {
        let snapshot: Vec<(i32, u8)> = overrides().iter().map(|(k, v)| (*k, *v)).collect();
        if snapshot.is_empty() {
            return 0;
        }
        let key = save_key();
        for (idx, level) in snapshot {
            if !apply_level(l, idx, level, key.as_deref()) {
                overrides().remove(&idx);
            }
        }
        0
    }
}

#[cfg(feature = "server")]
/// Hook "EntityRemoved"(ent, fullUpdate): forget the override immediately so a
/// new entity reusing the index never inherits it.
unsafe extern "C-unwind" fn l_entity_removed(l: LuaState) -> c_int {
    unsafe {
        if let Some(idx) = ent_index(l, 1) {
            overrides().remove(&idx);
        }
        0
    }
}

// ---------------------------------------------------------------------------
// Module entry points
// ---------------------------------------------------------------------------

unsafe fn set_fn(l: LuaState, table: c_int, name: &CStr, f: LuaFn) {
    unsafe {
        let a = api();
        (a.pushcclosure)(l, f, 0);
        (a.setfield)(l, table, name.as_ptr());
    }
}

#[cfg(feature = "server")]
unsafe fn set_nil(l: LuaState, table: c_int, name: &CStr) {
    unsafe {
        let a = api();
        (a.pushnil)(l);
        (a.setfield)(l, table, name.as_ptr());
    }
}

#[cfg(feature = "server")]
unsafe fn hook_add(l: LuaState, event: &CStr, f: LuaFn) {
    unsafe {
        let a = api();
        let base = (a.gettop)(l);
        (a.getfield)(l, LUA_GLOBALSINDEX, c"hook".as_ptr());
        (a.getfield)(l, base + 1, c"Add".as_ptr());
        (a.pushstring)(l, event.as_ptr());
        (a.pushstring)(l, HOOK_ID.as_ptr());
        (a.pushcclosure)(l, f, 0);
        let _ = (a.pcall)(l, 3, 0, 0);
        (a.settop)(l, base);
    }
}

#[cfg(feature = "server")]
unsafe fn hook_remove(l: LuaState, event: &CStr) {
    unsafe {
        let a = api();
        let base = (a.gettop)(l);
        (a.getfield)(l, LUA_GLOBALSINDEX, c"hook".as_ptr());
        (a.getfield)(l, base + 1, c"Remove".as_ptr());
        (a.pushstring)(l, event.as_ptr());
        (a.pushstring)(l, HOOK_ID.as_ptr());
        let _ = (a.pcall)(l, 2, 0, 0);
        (a.settop)(l, base);
    }
}

#[cfg(feature = "server")]
/// Pushes the Entity metatable and returns true, or leaves the stack unchanged.
unsafe fn push_entity_meta(l: LuaState) -> bool {
    unsafe {
        let a = api();
        let base = (a.gettop)(l);
        (a.getfield)(l, LUA_GLOBALSINDEX, c"FindMetaTable".as_ptr());
        (a.pushstring)(l, c"Entity".as_ptr());
        if (a.pcall)(l, 1, 1, 0) == 0 && (a.type_)(l, base + 1) == LUA_TTABLE {
            true
        } else {
            (a.settop)(l, base);
            false
        }
    }
}

#[no_mangle]
pub unsafe extern "C-unwind" fn gmod13_open(l: LuaState) -> c_int {
    unsafe {
        let Some(loaded) = load_api() else {
            return 0;
        };
        let _ = API.set(loaded);
        #[cfg(all(
            any(windows, target_os = "linux"),
            target_arch = "x86_64",
            feature = "client"
        ))]
        if running_as_client() {
            return open_client(l);
        }
        #[cfg(feature = "server")]
        {
            open_server(l)
        }
        #[cfg(not(feature = "server"))]
        {
            let _ = l;
            0
        }
    }
}

/// True when this DLL is the client module. With both features built in that is decided
/// from the file name (gmcl_*); a build with only one feature has no choice to make.
#[cfg(all(
    any(windows, target_os = "linux"),
    target_arch = "x86_64",
    feature = "client"
))]
#[cfg_attr(not(feature = "server"), allow(clippy::missing_const_for_fn))] // only const without the name check
unsafe fn running_as_client() -> bool {
    #[cfg(feature = "server")]
    unsafe {
        client_module::is_client_module()
    }
    #[cfg(not(feature = "server"))]
    true
}

/// Loaded as `gmcl_waterlevelercore`: the client's water. The Lua side gives it the
/// volumes (`AddVolume` etc.); its engine trace is
/// patched so the client sees and hears them like the server does.
#[cfg(all(
    any(windows, target_os = "linux"),
    target_arch = "x86_64",
    feature = "client"
))]
unsafe fn open_client(l: LuaState) -> c_int {
    unsafe {
        let a = api();
        let base = (a.gettop)(l);
        {
            let trace = match hook_state::install_client_contents() {
                Ok(note) => note,
                Err(why) => format!("trace hook failed: {why}"),
            };
            *HOOK_STATUS
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = format!("client: {trace}");
            (a.createtable)(l, 0, 8);
            let t = base + 1;
            set_fn(l, t, c"AddVolume", l_add_volume);
            set_fn(l, t, c"RemoveVolume", l_remove_volume);
            set_fn(l, t, c"ClearVolumes", l_clear_volumes);
            set_fn(l, t, c"VolumeCount", l_volume_count);
            set_fn(l, t, c"PointInVolume", l_point_in_volume);
            set_fn(l, t, c"IsContentsHooked", l_is_contents_hooked);
            set_fn(l, t, c"ContentsHits", l_contents_hits);
            set_fn(l, t, c"TraceHits", l_trace_hits);
            set_fn(l, t, c"SetDebug", l_set_debug);
            set_fn(l, t, c"TakeDebugLog", l_take_debug_log);
            set_fn(l, t, c"HookStatus", l_hook_status);
            (a.setfield)(l, LUA_GLOBALSINDEX, c"waterlevelercore".as_ptr());
            (a.settop)(l, base);
            0
        }
    }
}

/// Loaded as `gmsv_waterlevelercore`: the server's water, the overrides and the fluids.
#[cfg(feature = "server")]
unsafe fn open_server(l: LuaState) -> c_int {
    unsafe {
        let a = api();
        let base = (a.gettop)(l);

        // Global `waterleveler` table.
        (a.createtable)(l, 0, 16);
        let t = base + 1;
        set_fn(l, t, c"AddVolume", l_add_volume);
        set_fn(l, t, c"RemoveVolume", l_remove_volume);
        set_fn(l, t, c"ClearVolumes", l_clear_volumes);
        set_fn(l, t, c"VolumeCount", l_volume_count);
        set_fn(l, t, c"PointInVolume", l_point_in_volume);
        set_fn(l, t, c"IsContentsHooked", l_is_contents_hooked);
        set_fn(l, t, c"ContentsHits", l_contents_hits);
        set_fn(l, t, c"TraceHits", l_trace_hits);
        set_fn(l, t, c"SetDebug", l_set_debug);
        set_fn(l, t, c"TakeDebugLog", l_take_debug_log);
        set_fn(l, t, c"BubbleClips", l_bubble_clips);
        set_fn(l, t, c"ExplosionFixes", l_explosion_fixes);
        set_fn(l, t, c"ExplosionsRedirected", l_explosions_redirected);
        set_fn(l, t, c"AttachFluid", l_attach_fluid);
        set_fn(l, t, c"DetachFluid", l_detach_fluid);
        set_fn(l, t, c"FluidCount", l_fluid_count);
        set_fn(l, t, c"Set", l_set);
        set_fn(l, t, c"Get", l_get);
        set_fn(l, t, c"Clear", l_clear);
        set_fn(l, t, c"ClearAll", l_clear_all);
        set_fn(l, t, c"SetSaveKey", l_set_save_key);
        set_fn(l, t, c"IsHooked", l_is_hooked);
        set_fn(l, t, c"HookStatus", l_hook_status);
        set_fn(l, t, c"HookHits", l_hook_hits);
        (a.setfield)(l, LUA_GLOBALSINDEX, c"waterlevelercore".as_ptr()); // pops the table

        // Entity metatable: methods + Entity:WaterLevel wrapper.
        if push_entity_meta(l) {
            let mt = base + 1;

            // Keep the original across module reloads.
            (a.getfield)(l, mt, ORIG_KEY.as_ptr());
            let have_orig = (a.type_)(l, -1) != LUA_TNIL;
            (a.settop)(l, mt);
            if !have_orig {
                (a.getfield)(l, mt, c"WaterLevel".as_ptr());
                (a.setfield)(l, mt, ORIG_KEY.as_ptr()); // pops
            }
            (a.getfield)(l, mt, ORIG_KEY.as_ptr());
            (a.pushcclosure)(l, l_wrapped_water_level, 1); // pops the upvalue
            (a.setfield)(l, mt, c"WaterLevel".as_ptr());

            set_fn(l, mt, c"SetWaterLevelOverride", l_set);
            set_fn(l, mt, c"GetWaterLevelOverride", l_get);
            set_fn(l, mt, c"ClearWaterLevelOverride", l_clear);
            (a.settop)(l, base);
        }

        hook_add(l, c"Tick", l_tick);
        hook_add(l, c"EntityRemoved", l_entity_removed);

        // Engine hook (best effort; the Lua layers work without it).
        let water_level = hook_state::install();
        let contents = hook_state::install_contents();
        // A hook or patch that failed to install is always reported (where they exist at all:
        // the Windows and Linux servers). The all-good status is only printed with `developer` above 0; it is
        // always available from `waterleveler_status`.
        // Parts of the volumes that did not install are noted in parentheses.
        let failed = cfg!(any(windows, target_os = "linux"))
            && (water_level.is_err() || contents.as_ref().map_or(true, |what| what.contains(" (")));
        let water_level = match water_level {
            Ok(rva) => format!("hooked CBaseEntity::SetWaterLevel at server module+{rva:#x}"),
            Err(why) => format!("engine hook inactive: {why}"),
        };
        let contents = match contents {
            Ok(what) => format!("water volumes active via {what}"),
            Err(why) => format!("water volumes inactive: {why}"),
        };
        let status = format!("{water_level}; {contents}");
        if failed || developer_level(l) > 0.0 {
            lua_print(l, &format!("[wl] {status}"));
        }
        *HOOK_STATUS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = status;

        (a.settop)(l, base);
        0
    }
}

#[no_mangle]
#[cfg_attr(
    not(any(feature = "server", windows)),
    allow(unused_unsafe) // a client-only build has nothing unsafe to do off Windows
)]
pub unsafe extern "C-unwind" fn gmod13_close(l: LuaState) -> c_int {
    unsafe {
        if API.get().is_none() {
            return 0;
        }
        #[cfg(all(
            any(windows, target_os = "linux"),
            target_arch = "x86_64",
            feature = "client"
        ))]
        if running_as_client() {
            return close_client(l);
        }
        #[cfg(feature = "server")]
        {
            close_server(l)
        }
        #[cfg(not(feature = "server"))]
        {
            let _ = l;
            0
        }
    }
}

#[cfg(all(
    any(windows, target_os = "linux"),
    target_arch = "x86_64",
    feature = "client"
))]
unsafe fn close_client(l: LuaState) -> c_int {
    unsafe {
        let a = api();
        let base = (a.gettop)(l);
        hook_state::uninstall_contents();
        engine_hook::CLIENT_ENTITY_LIST.store(0, std::sync::atomic::Ordering::SeqCst);
        volumes().clear();
        (a.pushnil)(l);
        (a.setfield)(l, LUA_GLOBALSINDEX, c"waterlevelercore".as_ptr());
        (a.settop)(l, base);
        0
    }
}

#[cfg(feature = "server")]
unsafe fn close_server(l: LuaState) -> c_int {
    unsafe {
        let a = api();
        let base = (a.gettop)(l);

        // Fluid controllers hold engine objects; free them while the module is still loaded.
        hook_state::fluid_detach_all();
        // Remove the detour first so the engine never calls into unloaded code.
        hook_state::uninstall();
        hook_state::uninstall_contents();
        overrides().clear();
        volumes().clear();

        hook_remove(l, c"Tick");
        hook_remove(l, c"EntityRemoved");

        if push_entity_meta(l) {
            let mt = base + 1;
            (a.getfield)(l, mt, ORIG_KEY.as_ptr());
            if (a.type_)(l, -1) == LUA_TNIL {
                (a.settop)(l, mt);
            } else {
                (a.setfield)(l, mt, c"WaterLevel".as_ptr()); // pops
            }
            set_nil(l, mt, ORIG_KEY);
            set_nil(l, mt, c"SetWaterLevelOverride");
            set_nil(l, mt, c"GetWaterLevelOverride");
            set_nil(l, mt, c"ClearWaterLevelOverride");
            (a.settop)(l, base);
        }

        (a.pushnil)(l);
        (a.setfield)(l, LUA_GLOBALSINDEX, c"waterlevelercore".as_ptr());

        (a.settop)(l, base);
        0
    }
}

// ---------------------------------------------------------------------------
// Tests (x86-64): exercise the real hook machinery against the real
// bytes of SetWaterLevel taken from a 64-bit Windows server.dll (these run on a Linux host).
//   SERVER_DLL_X64=/path/to/server.dll cargo test --release
// The Linux server's signatures and patches are checked against a real server_srv.so:
//   SERVER_SO_X64=/path/to/server_srv.so cargo test --release
// ---------------------------------------------------------------------------

#[cfg(all(
    test,
    target_os = "linux",
    target_arch = "x86_64",
    feature = "server",
    feature = "client"
))]
mod tests;
