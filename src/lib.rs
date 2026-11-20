// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67
//! `gmsv_waterlevel` - per-entity water level overrides for Garry's Mod (server).
//!
//! Lua API (global table `waterleveler`, plus methods on the Entity metatable):
//!
//! ```lua
//! require("waterlevelercore")   -- raw module; the `waterleveler` Lua library wraps it
//! waterlevel.Set(ent, 3)        -- or ent:SetWaterLevelOverride(3)
//! waterlevel.Get(ent)           -- or ent:GetWaterLevelOverride()  -> number | nil
//! waterlevel.Clear(ent)         -- or ent:ClearWaterLevelOverride()
//! waterlevel.ClearAll()
//! waterlevel.IsHooked()         -- true if the engine hook is active
//! waterlevel.HookStatus()       -- human readable hook status
//! waterlevel.HookHits()         -- how many engine calls were overridden
//! waterlevel.SetSaveKey("m_nWaterLevel") -- datamap key written every tick; "" disables
//!
//! -- Water volumes (Windows x64): world-space boxes the engine treats as water
//! local id = waterlevel.AddVolume(x1, y1, z1, x2, y2, z2)
//! waterlevel.RemoveVolume(id)
//! waterlevel.ClearVolumes()
//! waterlevel.VolumeCount()
//! waterlevel.PointInVolume(x, y, z)  -- pure lookup
//! waterlevel.IsContentsHooked()
//! waterlevel.ContentsHits()          -- engine point queries answered with water
//! ```
//!
//! Water volumes work by swapping vtable slot 0 of the engine's `IEngineTrace`
//! object (`GetPointContents`) so points inside a box also report
//! `CONTENTS_WATER`. No machine code is patched for this one.
//!
//! Three layers:
//!   1. Engine hook (Windows x64 `server.dll` only): `CBaseEntity::SetWaterLevel`
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
// The engine hooks are only wired up on Windows x64; on other targets that
// code is compiled (and unit-tested) but not called.
#![cfg_attr(not(windows), allow(dead_code))]

use std::collections::BTreeMap;
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

type LuaState = *mut c_void;
type LuaFn = unsafe extern "C-unwind" fn(LuaState) -> c_int;

const LUA_GLOBALSINDEX: c_int = -10002;
const LUA_MULTRET: c_int = -1;
const LUA_TNONE: c_int = -1;
const LUA_TNIL: c_int = 0;
const LUA_TSTRING: c_int = 4;
const LUA_TNUMBER: c_int = 3;
const LUA_TTABLE: c_int = 5;
const LUA_TUSERDATA: c_int = 7;

const DEFAULT_SAVE_KEY: &str = "m_nWaterLevel";
const HOOK_ID: &CStr = c"gmsv_waterlevel";
const ORIG_KEY: &CStr = c"__waterlevel_orig";

// ---------------------------------------------------------------------------
// Lua C API (resolved at runtime)
// ---------------------------------------------------------------------------

struct Api {
    gettop: unsafe extern "C-unwind" fn(LuaState) -> c_int,
    settop: unsafe extern "C-unwind" fn(LuaState, c_int),
    pushvalue: unsafe extern "C-unwind" fn(LuaState, c_int),
    pushnil: unsafe extern "C-unwind" fn(LuaState),
    pushnumber: unsafe extern "C-unwind" fn(LuaState, f64),
    pushboolean: unsafe extern "C-unwind" fn(LuaState, c_int),
    pushstring: unsafe extern "C-unwind" fn(LuaState, *const c_char) -> *const c_char,
    pushcclosure: unsafe extern "C-unwind" fn(LuaState, LuaFn, c_int),
    createtable: unsafe extern "C-unwind" fn(LuaState, c_int, c_int),
    getfield: unsafe extern "C-unwind" fn(LuaState, c_int, *const c_char),
    setfield: unsafe extern "C-unwind" fn(LuaState, c_int, *const c_char),
    call: unsafe extern "C-unwind" fn(LuaState, c_int, c_int),
    pcall: unsafe extern "C-unwind" fn(LuaState, c_int, c_int, c_int) -> c_int,
    tonumber: unsafe extern "C-unwind" fn(LuaState, c_int) -> f64,
    toboolean: unsafe extern "C-unwind" fn(LuaState, c_int) -> c_int,
    tolstring: unsafe extern "C-unwind" fn(LuaState, c_int, *mut usize) -> *const c_char,
    type_: unsafe extern "C-unwind" fn(LuaState, c_int) -> c_int,
    touserdata: unsafe extern "C-unwind" fn(LuaState, c_int) -> *mut c_void,
}

static API: OnceLock<Api> = OnceLock::new();

#[inline]
fn api() -> &'static Api {
    // Only reachable after gmod13_open stored the table.
    API.get().expect("Lua API not loaded")
}

#[cfg(target_os = "linux")]
unsafe fn find_symbol(name: &CStr) -> *mut c_void {
    unsafe {
        #[link(name = "dl")]
        extern "C" {
            fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
            fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
        }
        const RTLD_LAZY: c_int = 0x1;
        const RTLD_NOLOAD: c_int = 0x4;

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
compile_error!("gmsv_waterlevel supports Linux and Windows only");

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
                    return None;
                }
                std::mem::transmute::<*mut c_void, _>(sym)
            }};
        }
        Some(Api {
            gettop: load!("lua_gettop"),
            settop: load!("lua_settop"),
            pushvalue: load!("lua_pushvalue"),
            pushnil: load!("lua_pushnil"),
            pushnumber: load!("lua_pushnumber"),
            pushboolean: load!("lua_pushboolean"),
            pushstring: load!("lua_pushstring"),
            pushcclosure: load!("lua_pushcclosure"),
            createtable: load!("lua_createtable"),
            getfield: load!("lua_getfield"),
            setfield: load!("lua_setfield"),
            call: load!("lua_call"),
            pcall: load!("lua_pcall"),
            tonumber: load!("lua_tonumber"),
            toboolean: load!("lua_toboolean"),
            tolstring: load!("lua_tolstring"),
            type_: load!("lua_type"),
            touserdata: load!("lua_touserdata"),
        })
    }
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// Entity index -> overridden water level (0..=3).
static OVERRIDES: Mutex<BTreeMap<i32, u8>> = Mutex::new(BTreeMap::new());
/// `None` = default key, `Some("")` = engine writes disabled.
static SAVE_KEY: Mutex<Option<String>> = Mutex::new(None);

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

/// How many entries `OVERRIDES` has, kept up to date by `OverridesGuard`, so the
/// water-level hook (called for every entity) can skip the lock when there are none.
static OVERRIDE_COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// The overrides map, locked. When it is released the entry count is published.
struct OverridesGuard(std::sync::MutexGuard<'static, BTreeMap<i32, u8>>);

impl std::ops::Deref for OverridesGuard {
    type Target = BTreeMap<i32, u8>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for OverridesGuard {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Drop for OverridesGuard {
    fn drop(&mut self) {
        // Runs before the mutex is released, so no reader sees a count that is
        // older than the map it could then lock.
        OVERRIDE_COUNT.store(self.0.len(), std::sync::atomic::Ordering::Relaxed);
    }
}

fn overrides() -> OverridesGuard {
    OverridesGuard(
        OVERRIDES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    )
}

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
#[cfg(test)]
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
#[cfg(test)]
fn contents_at(vols: &[Volume], p: [f32; 3]) -> c_int {
    vols.iter()
        .filter(|v| (0..3).all(|i| p[i] >= v.min[i] && p[i] <= v.max[i]))
        .fold(0, |c, v| c | v.contents)
}

#[cfg(test)]
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
mod engine_hook {
    // The Windows-only code below needs more of the parent's names than the Linux build uses.
    #[allow(clippy::wildcard_imports)]
    use super::*;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

    /// Bytes overwritten at the function entry (14-byte absolute jmp + 1 nop).
    pub(crate) const STOLEN: usize = 15;
    pub(crate) const TRAMPOLINE_SIZE: usize = STOLEN + 14;
    pub(crate) const OFF_PEV: usize = 0x30;
    pub(crate) const OFF_EDICT_INDEX: usize = 6;

    /// Signature of the function. `??` bytes are the vtable displacement of
    /// the `NetworkStateChanged` call. The 0x2ab offsets are deliberately NOT
    /// wildcarded: they double as a check that the datamap layout matches.
    pub(crate) const SIG: &str = "48 89 5C 24 08 48 89 6C 24 10 48 89 74 24 18 57 \
                           48 83 EC 20 8B FA 0F B6 A9 AB 02 00 00 48 8B F1 \
                           38 91 AB 02 00 00 74 17 48 8B 01 48 8D 91 AB 02 \
                           00 00 FF 90 ?? ?? 00 00 40 88 BE AB 02 00 00";

    pub(crate) static TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);
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

    /// `jmp [rip+0]; dq hook; nop` - 15 bytes written over the function entry.
    pub(crate) fn build_detour(hook: u64) -> [u8; STOLEN] {
        let mut b = [0x90u8; STOLEN];
        b[0] = 0xFF;
        b[1] = 0x25;
        b[2..6].copy_from_slice(&0u32.to_le_bytes());
        b[6..14].copy_from_slice(&hook.to_le_bytes());
        b
    }

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

    /// The replacement for `CBaseEntity::SetWaterLevel`. Windows x64 calling
    /// convention on every OS, because the original is a Windows function.
    pub(crate) unsafe extern "win64" fn hook_set_water_level(this: *mut c_void, level: c_int) {
        unsafe {
            let original: unsafe extern "win64" fn(*mut c_void, c_int) =
                std::mem::transmute(TRAMPOLINE.load(Ordering::Relaxed));
            let mut level = level;
            if !this.is_null() {
                let pev = *(this as *const u8).add(OFF_PEV).cast::<*const u8>();
                if !pev.is_null() {
                    let idx = i32::from(*pev.add(OFF_EDICT_INDEX).cast::<i16>());
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

    /// The handful of OS services needed to patch code.
    pub(crate) struct Platform {
        pub alloc_exec: unsafe fn(usize) -> *mut u8,
        /// Makes the range writable; returns the previous protection.
        pub make_writable: unsafe fn(*mut u8, usize) -> Option<u32>,
        pub restore_protect: unsafe fn(*mut u8, usize, u32),
        pub flush: unsafe fn(*mut u8, usize),
    }

    /// `CBaseEntity::UpdateWaterState` - used only to locate the engine's
    /// `enginetrace` pointer: the `mov rcx,[rip+disp32]` inside it (disp32 at
    /// byte 91) loads the `IEngineTrace` object whose vtable slot 0 is
    /// `GetPointContents(const Vector&, ...)`; the result is tested against
    /// `MASK_WATER` (0x4030) to set water level 1, 2 and 3.
    pub(crate) const UPDATE_WATER_STATE_SIG: &str = "\
        48 8B C4 53 48 83 EC 50 48 8B D9 0F 29 70 E8 48 81 C1 E8 01 00 00 48 C7 40 CC 00 00 00 00 \
        4C 8D 40 C8 C7 40 C8 00 00 00 00 48 8D 50 D8 C7 40 D8 00 00 00 3F 0F 57 F6 \
        48 C7 40 DC 00 00 00 3F E8 ?? ?? ?? ?? 33 D2 48 8B CB E8 ?? ?? ?? ?? 33 D2 48 8B CB \
        E8 ?? ?? ?? ?? 48 8B 0D ?? ?? ?? ?? 48 8D 54 24 20 45 33 C0 48 8B 01 FF 10 A9 30 40 00 00";
    pub(crate) const TRACE_GLOBAL_DISP_OFFSET: usize = 91;

    /// client.dll: `C_BaseEntity::UpdateWaterState` loads the client's `enginetrace`
    /// the same way (`mov rcx,[rip+disp32]`, `GetPointContents`, test `MASK_WATER`). The
    /// pattern is only the load and the test, so it does not depend on the function's
    /// frame; disp32 is at byte 3 of the match.
    pub(crate) const CLIENT_TRACE_SIG: &str =
        "48 8B 0D ?? ?? ?? ?? 48 8D 54 24 ?? 45 33 C0 48 8B 01 FF 10 A9 30 40 00 00";

    /// The `enginetrace` pointer variable loaded by the instruction at `at`.
    pub(crate) const unsafe fn trace_global_from_load(at: *const u8) -> *const *mut c_void {
        unsafe {
            let disp = at.add(3).cast::<i32>().read_unaligned() as isize;
            at.offset(7 + disp).cast::<*mut c_void>()
        }
    }

    /// `IClientEntityList` (client realm). Its vtable slot 3 is GetClientEntity(int).
    pub(crate) static CLIENT_ENTITY_LIST: AtomicUsize = AtomicUsize::new(0);
    pub(crate) const CLIENT_GET_ENTITY_SLOT: usize = 3;

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

    /// Replacement for `IEngineTrace::GetPointContents` (vtable slot 0). All
    /// four register arguments are forwarded untouched, so it does not matter
    /// whether the engine's signature has a mask parameter or not.
    pub(crate) unsafe extern "win64" fn hook_get_point_contents(
        this: *mut c_void,
        pos: *const f32,
        a3: usize,
        a4: usize,
    ) -> c_int {
        unsafe {
            let original: unsafe extern "win64" fn(*mut c_void, *const f32, usize, usize) -> c_int =
                std::mem::transmute(ORIGINAL_GPC.load(Ordering::Relaxed));
            let mut contents = original(this, pos, a3, a4);
            if !pos.is_null() {
                // Never block inside an engine callback.
                let p = [pos.read(), pos.add(1).read(), pos.add(2).read()];
                if let Some(vols) = snapshot_if(|v| v.may_contain(p)) {
                    let inside = vols.contents_at(p);
                    if inside != 0 {
                        contents |= inside;
                        bump(&VOLUME_HITS);
                    }
                }
            }
            contents
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

    /// Offset of `Ray_t::m_IsSwept`. The older 64-bit builds have it at +0x41
    /// (`m_IsRay` +0x40); the x86-64 branch inserted an 8-byte member (the world
    /// axis transform pointer) at +0x40, so the flags sit at +0x48/+0x49.
    pub(crate) const RAY_SWEPT_OFFSET_OLD: usize = 0x41;
    pub(crate) static RAY_SWEPT_OFFSET: AtomicUsize = AtomicUsize::new(RAY_SWEPT_OFFSET_OLD);

    /// The engine helper that builds a `Ray_t` on its stack from two points
    /// (`UTIL_TraceLine`-style): its `mov byte [rax+disp],1` stores `m_IsSwept`.
    /// The frame is `mov rax,rsp; push rbx; sub rsp,0x90` with the `Ray_t` at
    /// rsp+0x40, so offset = disp + 0x98 - 0x40. Byte 53 is disp.
    pub(crate) const RAY_HELPER_SIG: &str = "48 8B C4 53 48 81 EC 90 00 00 00 F3 0F 10 42 08 F3 0F 10 19 \
        F3 0F 10 0A F3 0F 10 61 04 F3 0F 5C CB F3 0F 10 52 04 F3 0F 10 69 08 F3 0F 5C D4 F3 0F 5C C5 \
        C6 40 ?? 01";
    pub(crate) const RAY_HELPER_DISP_BYTE: usize = 53;

    pub(crate) const unsafe fn ray_swept_offset_from(func: *const u8) -> usize {
        unsafe {
            let disp = func.add(RAY_HELPER_DISP_BYTE).cast::<i8>().read() as isize;
            (disp + 0x98 - 0x40) as usize
        }
    }

    /// Traces seen with a water bit in the mask (diagnostics).
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
    pub(crate) fn segment_exit_fraction(
        s: [f32; 3],
        d: [f32; 3],
        lo: [f32; 3],
        hi: [f32; 3],
    ) -> f32 {
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
        /// The ray enters a volume from outside.
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
        let (t, axis, min_face, vi) = best?;
        let v = &vols[vi];
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
        let Some((lo, hi)) = bounds else {
            return true; // no volumes
        };
        unsafe {
            let rf = |o: usize| ray.add(o).cast::<f32>().read_unaligned();
            let s = [rf(0x00), rf(0x04), rf(0x08)];
            let d = [rf(0x10), rf(0x14), rf(0x18)];
            let ext = [rf(0x30), rf(0x34), rf(0x38)];
            (0..3).any(|i| {
                let e = s[i] + d[i];
                let (smin, smax) = if e < s[i] { (e, s[i]) } else { (s[i], e) };
                smax < lo[i] - ext[i] || smin > hi[i] + ext[i]
            })
        }
    }

    /// Replacement for `IEngineTrace::TraceRay`.
    pub(crate) unsafe extern "win64" fn hook_trace_ray(
        this: *mut c_void,
        ray: *const u8,
        mask: u32,
        filter: *mut c_void,
        trace: *mut u8,
    ) {
        unsafe {
            let original: unsafe extern "win64" fn(
                *mut c_void,
                *const u8,
                u32,
                *mut c_void,
                *mut u8,
            ) = std::mem::transmute(ORIGINAL_TRACE_RAY.load(Ordering::Relaxed));
            original(this, ray, mask, filter, trace);
            if mask & ((CONTENTS_WATER | CONTENTS_SLIME) as u32) == 0
                || ray.is_null()
                || trace.is_null()
            {
                return;
            }
            // Never block inside an engine callback.
            // A ray far from every volume changes nothing (and needs no world entity).
            if let Some(vols) = snapshot_if(|v| !ray_misses_bounds(v.bounds, ray)) {
                // Only the volumes whose contents the trace's mask asks for are considered.
                if apply_water_to_trace_masked(&vols, ray, trace, world_entity(), mask) {
                    bump(&TRACE_HITS);
                }
            }
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

    pub(crate) unsafe fn uninstall_slot_hook(
        entry: usize,
        hook: usize,
        original: usize,
        p: &Platform,
    ) {
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

    /// Walks the executable sections of a loaded PE image (`base` = module
    /// base) and returns the single match of [`SIG`].
    pub(crate) unsafe fn scan_image(base: *const u8) -> Result<*mut u8, &'static str> {
        unsafe { scan_sig(base, SIG) }
    }

    /// `83 F9 FF  74 ??  48 8B 35 <entity array>  8B C1  25 FF 3F 00 00
    /// C1 E9 0E  48 C1 E0 05  39 4C 30 10`: the engine's "entity from handle"
    /// idiom (entries are 32 bytes: pointer at +8, serial at +0x10). It occurs
    /// many times, always with the same array global.
    pub(crate) const ENTITY_ARRAY_SIG: &str =
        "83 F9 FF 74 ?? 48 8B 35 ?? ?? ?? ?? 8B C1 25 FF 3F 00 00 \
                                        C1 E9 0E 48 C1 E0 05 39 4C 30 10";
    pub(crate) static ENTITY_ARRAY_GLOBAL: AtomicUsize = AtomicUsize::new(0);

    /// Address of the global holding the entity array, if every match of the
    /// idiom agrees on it.
    pub(crate) unsafe fn entity_array_global(base: *const u8) -> Result<usize, &'static str> {
        unsafe {
            let all = scan_sig_all(base, ENTITY_ARRAY_SIG)?;
            let mut global = 0usize;
            for at in all {
                let disp = (at.add(8) as *const i32).read_unaligned() as isize;
                let g = at.offset(12 + disp) as usize;
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
    pub(crate) unsafe fn world_entity() -> usize {
        unsafe {
            let list = CLIENT_ENTITY_LIST.load(Ordering::Relaxed);
            if list != 0 {
                // Client realm: entity 0 of the client's entity list.
                let vtable = (list as *const *const usize).read();
                let get: unsafe extern "win64" fn(usize, c_int) -> usize =
                    std::mem::transmute(vtable.add(CLIENT_GET_ENTITY_SLOT).read());
                return get(list, 0);
            }
            let g = ENTITY_ARRAY_GLOBAL.load(Ordering::Relaxed);
            if g == 0 {
                return 0;
            }
            let array = (g as *const usize).read_unaligned();
            if array == 0 {
                return 0;
            }
            ((array + 8) as *const usize).read_unaligned()
        }
    }

    /// Every match of a signature in the executable sections.
    pub(crate) unsafe fn scan_sig_all(
        base: *const u8,
        sig: &str,
    ) -> Result<Vec<*mut u8>, &'static str> {
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
            let pat = parse_sig(sig);
            let mut found: Vec<*mut u8> = Vec::new();
            for i in 0..nsec {
                let s = sections + i * 40;
                let vsize = rd32(s + 8) as usize;
                let va = rd32(s + 12) as usize;
                let chars = rd32(s + 36);
                if chars & 0x2000_0000 == 0 {
                    continue;
                }
                let region = std::slice::from_raw_parts(base.add(va), vsize);
                for off in find_all(region, &pat) {
                    found.push(base.add(va + off).cast_mut());
                }
            }
            if found.is_empty() {
                return Err("signature not found (different server.dll build?)");
            }
            Ok(found)
        }
    }

    /// Same, for any signature.
    pub(crate) unsafe fn scan_sig(base: *const u8, sig: &str) -> Result<*mut u8, &'static str> {
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
            let pat = parse_sig(sig);
            let mut found: Vec<*mut u8> = Vec::new();
            for i in 0..nsec {
                let s = sections + i * 40;
                let vsize = rd32(s + 8) as usize;
                let va = rd32(s + 12) as usize;
                let chars = rd32(s + 36);
                if chars & 0x2000_0000 == 0 {
                    continue; // not executable
                }
                let region = std::slice::from_raw_parts(base.add(va), vsize);
                for off in find_all(region, &pat) {
                    found.push(base.add(va + off).cast_mut());
                }
            }
            match found.len() {
                1 => Ok(found[0]),
                0 => Err("signature not found (different server.dll build?)"),
                _ => Err("signature is ambiguous"),
            }
        }
    }

    /// Detours `target`. Returns the original bytes for [`uninstall`].
    pub(crate) unsafe fn install(
        target: *mut u8,
        p: &Platform,
    ) -> Result<[u8; STOLEN], &'static str> {
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

    /// Finds `mov rcx,[physenv]; lea r8,[..]; mov rdx,rsi; mov r9,[rcx];
    /// call [r9+0x58]` in the world-setup code. The rip displacement of the
    /// first instruction is the `physenv` global.
    pub(crate) const PHYSENV_SIG: &str =
        "48 8B 0D ?? ?? ?? ?? 4C 8D 85 ?? ?? ?? ?? 48 8B D6 4C 8B 09 41 FF 51 58";
    pub(crate) const CREATE_FLUID_SLOT: usize = 11;
    pub(crate) const DESTROY_FLUID_SLOT: usize = 12;

    /// Address of the engine's `physenv` variable.
    pub(crate) const unsafe fn physenv_global_from(func: *const u8) -> *const *mut c_void {
        unsafe {
            let disp = func.add(3).cast::<i32>().read_unaligned() as isize;
            func.offset(7 + disp).cast::<*mut c_void>()
        }
    }

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

    type CreateFluidFn =
        unsafe extern "win64" fn(*mut c_void, *mut c_void, *const FluidParams) -> *mut c_void;
    type DestroyFluidFn = unsafe extern "win64" fn(*mut c_void, *mut c_void);

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

    /// `IPhysicsObject` vtable slots (confirmed in the world water setup:
    /// `call [rbx+0xb8]`, `or ax,0x200`, `call [rbx+0xb0]`).
    pub(crate) const OBJ_SET_CALLBACK_FLAGS_SLOT: usize = 0xb0 / 8;
    pub(crate) const OBJ_GET_CALLBACK_FLAGS_SLOT: usize = 0xb8 / 8;
    /// The engine ORs this into the world water object before creating its
    /// fluid controller (`CALLBACK_GLOBAL_TOUCH_STATIC`, unconfirmed name).
    pub(crate) const FLUID_OBJECT_CALLBACK_FLAG: u16 = 0x200;

    type GetFlagsFn = unsafe extern "win64" fn(*mut c_void) -> u16;
    type SetFlagsFn = unsafe extern "win64" fn(*mut c_void, u16);

    /// Mirrors the engine's setup of a water object. Returns the old flags.
    pub(crate) unsafe fn add_fluid_callback_flag(object: *mut c_void) -> u16 {
        unsafe {
            let vtable = (object as *const *const usize).read();
            let get: GetFlagsFn =
                std::mem::transmute(vtable.add(OBJ_GET_CALLBACK_FLAGS_SLOT).read());
            let set: SetFlagsFn =
                std::mem::transmute(vtable.add(OBJ_SET_CALLBACK_FLAGS_SLOT).read());
            let old = get(object);
            set(object, old | FLUID_OBJECT_CALLBACK_FLAG);
            old
        }
    }

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

    pub(crate) const FLUID_END_TOUCH_SIG: &str = "48 89 5C 24 10 48 89 7C 24 18 41 56 48 83 EC 20 \
        FF 81 F0 01 00 00 4D 8B F0 48 8B DA 48 8B F9 48 85 D2 0F 84 ?? ?? ?? ?? 4D 85 C0 74 ?? \
        48 8B 02 48 8B CA 48 89 74 24 30 FF 90 88 00 00 00 48 8B F0 48 85 C0 74 ?? \
        48 8B D0 48 8B CF E8 ?? ?? ?? ?? 0F 2F 05 ?? ?? ?? ?? 72 ?? 4C 8B C6 48 8B D3 49 8B CE \
        E8 ?? ?? ?? ?? 81 A6 90 01 00 00 FF FF F7 FF 49 8B CE 49 8B 06 48 8B 1E FF 50 30 \
        4C 63 C0 BA 01 00 00 00 48 8B CE FF 93 E8 02 00 00 FF 8F F0 01 00 00";
    /// Whole instructions covered by the 14-byte jump.
    pub(crate) const FLUID_END_TOUCH_STOLEN: usize = 16;
    pub(crate) static FLUID_END_TOUCH_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);
    /// `IPhysicsObject::GetGameData` (the entity), vtable offset 0x88.
    pub(crate) const OBJ_GET_GAME_DATA_SLOT: usize = 0x88 / 8;
    /// `CBaseEntity::m_iEFlags` and its `EFL_TOUCHING_FLUID` bit.
    pub(crate) const ENTITY_EFLAGS_OFFSET: usize = 0x190;
    pub(crate) const EFL_TOUCHING_FLUID: u32 = 0x80000;
    /// `CBaseEntity::OnEntityEvent(event`, data), vtable offset 0x2e8.
    pub(crate) const ENTITY_EVENT_SLOT: usize = 0x2e8 / 8;
    pub(crate) const ENTITY_EVENT_WATER_UNTOUCH: c_int = 1;

    /// Controllers destroyed by this module (address, contents) that
    /// vphysics objects may still point at.
    static DEAD_FLUIDS: Mutex<Vec<(usize, c_int)>> = Mutex::new(Vec::new());
    const DEAD_FLUIDS_MAX: usize = 256;

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

    /// A new controller may reuse a dead one's address.
    pub(crate) fn fluid_born(controller: usize) {
        DEAD_FLUIDS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|d| d.0 != controller);
    }

    pub(crate) fn forget_dead_fluids() {
        DEAD_FLUIDS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
    }

    pub(crate) unsafe extern "win64" fn hook_fluid_end_touch(
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
                let original: unsafe extern "win64" fn(*mut c_void, *mut c_void, *mut c_void) =
                    std::mem::transmute(FLUID_END_TOUCH_TRAMPOLINE.load(Ordering::Relaxed));
                original(this, object, fluid);
                return;
            };
            if object.is_null() {
                return;
            }
            let get_game_data: unsafe extern "win64" fn(*mut c_void) -> *mut u8 =
                std::mem::transmute(
                    *(*(object as *const *const usize)).add(OBJ_GET_GAME_DATA_SLOT),
                );
            let entity = get_game_data(object);
            if entity.is_null() {
                return;
            }
            let flags = entity.add(ENTITY_EFLAGS_OFFSET).cast::<u32>();
            flags.write_unaligned(flags.read_unaligned() & !EFL_TOUCHING_FLUID);
            let on_event: unsafe extern "win64" fn(*mut u8, c_int, *mut c_void) =
                std::mem::transmute(*(*(entity as *const *const usize)).add(ENTITY_EVENT_SLOT));
            on_event(
                entity,
                ENTITY_EVENT_WATER_UNTOUCH,
                contents as isize as *mut c_void,
            );
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

    pub(crate) const BUBBLE_TRAIL_SIG: &str =
        "48 8B C4 48 89 58 08 48 89 70 10 57 48 81 EC E0 00 00 00 \
                                        F3 0F 10 49 08 41 8B F0 0F 29 70 E8 0F 28 D1 \
                                        F3 0F 58 15 ?? ?? ?? ?? 0F 29 78 D8 48 8B FA \
                                        44 0F 29 48 B8 48 8B D9 E8";
    /// Whole instructions covered by the 14-byte jump.
    pub(crate) const BUBBLE_TRAIL_STOLEN: usize = 19;
    pub(crate) static BUBBLE_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);
    pub(crate) static BUBBLE_CLIPS: AtomicU64 = AtomicU64::new(0);

    /// A detour of arbitrary stolen length.
    pub(crate) struct Patch {
        pub target: usize,
        pub original: Vec<u8>,
        pub trampoline: usize,
    }

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
            let mut tb = original.clone();
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

    pub(crate) unsafe fn uninstall_patch(patch: &Patch, p: &Platform) {
        unsafe {
            let target = patch.target as *mut u8;
            if let Some(old) = (p.make_writable)(target, patch.original.len()) {
                std::ptr::copy_nonoverlapping(
                    patch.original.as_ptr(),
                    target,
                    patch.original.len(),
                );
                (p.restore_protect)(target, patch.original.len(), old);
                (p.flush)(target, patch.original.len());
            }
        }
    }

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

    #[derive(Debug, PartialEq)]
    pub(crate) enum BubbleClip {
        /// Call the engine function unchanged.
        Keep,
        /// Nothing worth drawing: skip the call.
        Skip,
        /// Call it with these end points and the count scaled by the fraction.
        Clipped([f32; 3], [f32; 3], f32),
    }

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

    /// Replacement for `UTIL_BubbleTrail` (Windows x64 convention).
    pub(crate) unsafe extern "win64" fn hook_bubble_trail(
        from: *const f32,
        to: *const f32,
        count: c_int,
    ) {
        unsafe {
            let original: unsafe extern "win64" fn(*const f32, *const f32, c_int) =
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
                // The lock must be gone before calling the original: it calls back
                // into the point-contents hook, which uses try_lock.
                if let Some(vols) = snapshot() {
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

    /// Per function: the prologue of each known build (tried in order).
    /// Newer builds store rbx at [rsp+0x10] and reshuffle the pushes.
    pub(crate) const EXPLOSION_TARGETS: [&[(&str, usize)]; 3] = [
        &[
            ("48 89 5C 24 08 55 48 8D 6C 24 A9 48 81 EC F0 00 00 00 80 B9 AB 02 00 00 02", 18),
            ("48 89 5C 24 10 55 48 8D 6C 24 A9 48 81 EC F0 00 00 00 80 B9 AB 02 00 00 02", 18),
        ],
        &[
            ("48 8B C4 48 89 58 08 48 89 70 10 48 89 78 18 4C 89 70 20 55 48 8D 68 D8 48 81 EC 20 01 00 00 0F 29 70 E8 48 8B D9 8B 89 80 02 00 00", 19),
            ("48 8B C4 48 89 58 10 48 89 70 18 55 57 41 56 48 8D 68 C8 48 81 EC 20 01 00 00 0F 29 70 D8 48 8B D9 8B 89 80 02 00 00", 15),
        ],
        &[
            ("48 89 5C 24 08 48 89 7C 24 10 55 48 8D 6C 24 A9 48 81 EC F0 00 00 00 80 B9 AB 02 00 00 00", 16),
            ("48 89 5C 24 10 48 89 7C 24 18 55 48 8D 6C 24 A9 48 81 EC F0 00 00 00 80 B9 AB 02 00 00 00", 16),
        ],
    ];

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
    pub(crate) static EXPLOSION_TRAMPOLINES: [AtomicUsize; 3] = [
        AtomicUsize::new(0),
        AtomicUsize::new(0),
        AtomicUsize::new(0),
    ];
    pub(crate) static EXPLOSION_FIXES: AtomicU64 = AtomicU64::new(0);
    pub(crate) const ENTITY_WATER_LEVEL_OFFSET: usize = 0x2ab;
    pub(crate) const ENTITY_ORIGIN_OFFSET: usize = 0x33c;

    /// Raises `this`'s water level to 2 if its origin is in a volume.
    pub(crate) unsafe fn raise_water_level_in_volume(this: *mut u8) {
        unsafe {
            if this.is_null() {
                return;
            }
            let o = this.add(ENTITY_ORIGIN_OFFSET) as *const f32;
            let p = [
                o.read_unaligned(),
                o.add(1).read_unaligned(),
                o.add(2).read_unaligned(),
            ];
            // try_lock: never wait (the original calls back into the contents hook).
            let inside = snapshot_if(|v| v.may_contain(p)).is_some_and(|v| v.contains(p));
            if inside {
                let level = this.add(ENTITY_WATER_LEVEL_OFFSET);
                if level.read() < 2 {
                    level.write(2);
                    bump(&EXPLOSION_FIXES);
                }
            }
        }
    }

    type ExplosionFn = unsafe extern "win64" fn(*mut u8, usize, usize, usize) -> usize;

    unsafe fn explosion_hook(i: usize, this: *mut u8, a: usize, b: usize, c: usize) -> usize {
        unsafe {
            raise_water_level_in_volume(this);
            let original: ExplosionFn =
                std::mem::transmute(EXPLOSION_TRAMPOLINES[i].load(Ordering::Relaxed));
            original(this, a, b, c)
        }
    }
    pub(crate) unsafe extern "win64" fn hook_explosion0(
        t: *mut u8,
        a: usize,
        b: usize,
        c: usize,
    ) -> usize {
        unsafe { explosion_hook(0, t, a, b, c) }
    }
    pub(crate) unsafe extern "win64" fn hook_explosion1(
        t: *mut u8,
        a: usize,
        b: usize,
        c: usize,
    ) -> usize {
        unsafe { explosion_hook(1, t, a, b, c) }
    }
    pub(crate) unsafe extern "win64" fn hook_explosion2(
        t: *mut u8,
        a: usize,
        b: usize,
        c: usize,
    ) -> usize {
        unsafe { explosion_hook(2, t, a, b, c) }
    }
    pub(crate) const EXPLOSION_HOOKS: [ExplosionFn; 3] =
        [hook_explosion0, hook_explosion1, hook_explosion2];

    /// DispatchEffect(const char *name, const `CEffectData` &data).
    /// Only its address is used (to send the water explosion effect), nothing is patched.
    pub(crate) const DISPATCH_EFFECT_SIG: &str = "48 89 5C 24 08 57 48 83 EC 70 48 8B D9 48 8B FA \
                                           48 8D 4C 24 30 E8 ?? ?? ?? ?? 48 8D 05 ?? ?? ?? ?? 48 8B D7";
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

    pub(crate) const TE_ORIGIN_OFFSET: usize = 0x18;
    pub(crate) static ORIGINAL_TE_CREATE: AtomicUsize = AtomicUsize::new(0);
    pub(crate) static DISPATCH_EFFECT_ADDR: AtomicUsize = AtomicUsize::new(0);
    pub(crate) static EXPLOSIONS_REDIRECTED: AtomicU64 = AtomicU64::new(0);

    /// Address of `CTEExplosion`'s vtable, found through RTTI only: the type
    /// descriptor name, the complete object locator that names it, and the
    /// pointer to that locator just before the vtable. No code bytes are
    /// involved, so it does not depend on how the compiler laid out any
    /// function.
    pub(crate) unsafe fn te_explosion_vtable(base: *const u8) -> Result<usize, &'static str> {
        unsafe { rtti_vtable(base, b".?AVCTEExplosion@@\0") }
    }

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

    /// Fills a `CEffectData` the way the engine does for the water explosion
    /// (flags 0x428, scale 128, magnitude 50; byte at +0x88 set).
    #[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
    pub(crate) fn water_explosion_effect_data(origin: [f32; 3]) -> [u8; 0xa0] {
        water_explosion_effect_data_with(origin, 0x428, 128.0, 50.0)
    }

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
            let inside = snapshot_if(|v| v.may_contain(origin))
                .is_some_and(|v| v.surface_z(origin).is_some());
            if !inside {
                return false;
            }
            // CTEExplosion fields: scale f32 @0x28, flags u32 @0x2c, magnitude i32 @0x34.
            // The effect gets the blast point: the client traces up from it to find
            // the surface and the depth (which picks the muffled sound).
            let scale = this.add(0x28).cast::<f32>().read_unaligned();
            let flags = this.add(0x2c).cast::<u32>().read_unaligned();
            let magnitude = this.add(0x34).cast::<i32>().read_unaligned() as f32;
            let data = water_explosion_effect_data_with(origin, flags, scale, magnitude);
            let f: unsafe extern "win64" fn(*const u8, *const u8) = std::mem::transmute(dispatch);
            f(c"WaterSurfaceExplosion".as_ptr().cast(), data.as_ptr());
            bump(&EXPLOSIONS_REDIRECTED);
            true
        }
    }

    /// `CBaseTempEntity::Create(this`, filter, delay): every temp entity goes
    /// through it (`CTempEnts::Explosion` jumps straight to it, so the vtable
    /// slot is never used). Only `CTEExplosion` objects are looked at.
    pub(crate) const TE_CREATE_SIG: &str =
        "48 89 5C 24 08 48 89 6C 24 10 48 89 74 24 18 57 48 83 EC 40 \
                                     48 8B 05 ?? ?? ?? ?? 48 8B EA 0F 29 74 24 30";
    pub(crate) const TE_CREATE_STOLEN: usize = 20;
    pub(crate) static TE_EXPLOSION_VTABLE: AtomicUsize = AtomicUsize::new(0);

    pub(crate) unsafe extern "win64" fn hook_te_create(
        this: *mut u8,
        filter: *mut c_void,
        delay: f32,
    ) {
        unsafe {
            let vtable = TE_EXPLOSION_VTABLE.load(Ordering::Relaxed);
            if vtable != 0
                && !this.is_null()
                && (this as *const usize).read() == vtable
                && redirect_explosion(this)
            {
                return;
            }
            let original: unsafe extern "win64" fn(*mut u8, *mut c_void, f32) =
                std::mem::transmute(ORIGINAL_TE_CREATE.load(Ordering::Relaxed));
            original(this, filter, delay);
        }
    }

    // ---- Windows glue (thin on purpose) ----------------------------------

    #[cfg(windows)]
    pub(crate) mod win {
        #[allow(clippy::wildcard_imports)]
        // the Windows-only code needs more names than the Linux build uses
        use super::*;

        #[link(name = "kernel32")]
        extern "system" {
            fn GetModuleHandleA(name: *const c_char) -> *mut c_void;
            fn VirtualAlloc(addr: *mut c_void, size: usize, ty: u32, prot: u32) -> *mut c_void;
            fn VirtualProtect(addr: *mut c_void, size: usize, new: u32, old: *mut u32) -> i32;
            fn FlushInstructionCache(process: *mut c_void, addr: *const c_void, size: usize)
                -> i32;
            fn GetCurrentProcess() -> *mut c_void;
        }
        const MEM_COMMIT_RESERVE: u32 = 0x3000;
        const PAGE_EXECUTE_READWRITE: u32 = 0x40;

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
        unsafe fn flush(p: *mut u8, n: usize) {
            unsafe {
                FlushInstructionCache(GetCurrentProcess(), p as *const c_void, n);
            }
        }

        pub(crate) const PLATFORM: Platform = Platform {
            alloc_exec,
            make_writable,
            restore_protect,
            flush,
        };

        pub(crate) unsafe fn server_base() -> *const u8 {
            unsafe { GetModuleHandleA(c"server.dll".as_ptr()) as *const u8 }
        }

        pub(crate) unsafe fn client_base() -> *const u8 {
            unsafe { GetModuleHandleA(c"client.dll".as_ptr()) as *const u8 }
        }

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
                let create: unsafe extern "win64" fn(*const c_char, *mut c_int) -> usize =
                    std::mem::transmute(create);
                create(name.as_ptr(), std::ptr::null_mut())
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Client realm (gmcl_waterlevelercore)
// ---------------------------------------------------------------------------
//
// Nothing about the water explosion is patched on the client: its effect finds the
// surface itself with a MASK_WATER trace up from the blast point and picks the
// underwater sound from that distance, so the client's hooked trace (see `engine_hook`)
// is all it needs.

#[cfg(all(windows, target_arch = "x86_64"))]
mod client_module {
    #[allow(clippy::wildcard_imports)]
    // the Windows-only code needs more names than the Linux build uses
    use super::*;

    /// True when this DLL was loaded as the client module (file name gmcl_*).
    pub(crate) unsafe fn is_client_module() -> bool {
        unsafe {
            #[link(name = "kernel32")]
            extern "system" {
                fn GetModuleHandleExA(
                    flags: u32,
                    addr: *const c_void,
                    out: *mut *mut c_void,
                ) -> i32;
                fn GetModuleFileNameA(module: *mut c_void, buf: *mut u8, size: u32) -> u32;
            }
            const FROM_ADDRESS_UNCHANGED: u32 = 0x4 | 0x2;
            let mut module: *mut c_void = std::ptr::null_mut();
            if GetModuleHandleExA(
                FROM_ADDRESS_UNCHANGED,
                is_client_module as *const c_void,
                &mut module,
            ) == 0
            {
                return false;
            }
            let mut buf = [0u8; 520];
            let n = GetModuleFileNameA(module, buf.as_mut_ptr(), buf.len() as u32) as usize;
            let path = String::from_utf8_lossy(&buf[..n.min(buf.len())]).to_lowercase();
            let name = path.rsplit(['\\', '/']).next().unwrap_or("");
            name.starts_with("gmcl_")
        }
    }
}

#[cfg(all(windows, target_arch = "x86_64"))]
mod hook_state {
    use super::engine_hook::{self, STOLEN};
    #[allow(clippy::wildcard_imports)]
    // the Windows-only code needs more names than the Linux build uses
    use super::*;

    pub(crate) static TARGET: Mutex<Option<(usize, [u8; STOLEN])>> = Mutex::new(None);

    pub(crate) unsafe fn install() -> Result<usize, &'static str> {
        unsafe {
            let base = engine_hook::win::server_base();
            if base.is_null() {
                return Err("server.dll is not loaded");
            }
            let target = engine_hook::scan_image(base)?;
            let original = engine_hook::install(target, &engine_hook::win::PLATFORM)?;
            *TARGET
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) =
                Some((target as usize, original));
            Ok(target as usize - base as usize)
        }
    }

    pub(crate) unsafe fn uninstall() {
        unsafe {
            let taken = TARGET
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            if let Some((target, original)) = taken {
                engine_hook::uninstall(target as *mut u8, &original, &engine_hook::win::PLATFORM);
            }
        }
    }

    pub(crate) fn active() -> bool {
        TARGET
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_some()
    }

    /// (vtable slot address, original function pointer)
    pub(crate) static CONTENTS: Mutex<Vec<(usize, usize, usize)>> = Mutex::new(Vec::new());

    /// Hooks `IEngineTrace::GetPointContents` via the engine object that
    /// `server.dll` itself uses (found through `UpdateWaterState`).
    #[allow(clippy::too_many_lines)]
    pub(crate) unsafe fn install_contents() -> Result<String, &'static str> {
        unsafe {
            let base = engine_hook::win::server_base();
            if base.is_null() {
                return Err("server.dll is not loaded");
            }
            let func = engine_hook::scan_sig(base, engine_hook::UPDATE_WATER_STATE_SIG)?;
            let trace = engine_hook::trace_global_from(func).read();
            let (slot, original) =
                engine_hook::install_contents_hook(trace, &engine_hook::win::PLATFORM)?;
            let mut hooks = vec![(
                slot,
                original,
                engine_hook::hook_get_point_contents as *const () as usize,
            )];

            // Ray_t layout: where this build keeps m_IsSwept.
            let ray_note = match engine_hook::scan_sig(base, engine_hook::RAY_HELPER_SIG) {
                Ok(f) => {
                    let off = engine_hook::ray_swept_offset_from(f);
                    engine_hook::RAY_SWEPT_OFFSET.store(off, std::sync::atomic::Ordering::SeqCst);
                    format!(" (Ray_t m_IsSwept at +{off:#x})")
                }
                Err(why) => {
                    engine_hook::RAY_SWEPT_OFFSET.store(
                        engine_hook::RAY_SWEPT_OFFSET_OLD,
                        std::sync::atomic::Ordering::SeqCst,
                    );
                    format!(" (Ray_t layout assumed old: {why})")
                }
            };

            // Second hook: traces. If it fails the point-contents hook stays.
            let trace_hook = engine_hook::hook_trace_ray as *const () as usize;
            let summary = match engine_hook::install_slot_hook(
                trace,
                engine_hook::TRACE_RAY_SLOT,
                trace_hook,
                &engine_hook::ORIGINAL_TRACE_RAY,
                &engine_hook::win::PLATFORM,
            ) {
                Ok((entry, orig)) => {
                    hooks.push((entry, orig, trace_hook));
                    format!("GetPointContents + TraceRay{ray_note}")
                }
                Err(why) => format!("GetPointContents only (TraceRay hook failed: {why})"),
            };
            *CONTENTS
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = hooks;

            // Third hook: bubble trails are clipped to the volume they are in.
            let bubble =
                match engine_hook::scan_sig(base, engine_hook::BUBBLE_TRAIL_SIG).and_then(|t| {
                    engine_hook::install_patch(
                        t,
                        engine_hook::BUBBLE_TRAIL_SIG,
                        engine_hook::BUBBLE_TRAIL_STOLEN,
                        engine_hook::hook_bubble_trail as *const () as usize,
                        &engine_hook::win::PLATFORM,
                    )
                }) {
                    Ok(patch) => {
                        engine_hook::BUBBLE_TRAMPOLINE
                            .store(patch.trampoline, std::sync::atomic::Ordering::SeqCst);
                        *BUBBLE
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(patch);
                        " + UTIL_BubbleTrail".to_string()
                    }
                    Err(why) => format!(" (bubble-trail clipping inactive: {why})"),
                };
            // Fluids destroyed while props are in them must not crash FluidEndTouch.
            let end_touch = match engine_hook::scan_sig(base, engine_hook::FLUID_END_TOUCH_SIG)
                .and_then(|t| {
                    engine_hook::install_patch(
                        t,
                        engine_hook::FLUID_END_TOUCH_SIG,
                        engine_hook::FLUID_END_TOUCH_STOLEN,
                        engine_hook::hook_fluid_end_touch as *const () as usize,
                        &engine_hook::win::PLATFORM,
                    )
                }) {
                Ok(patch) => {
                    engine_hook::FLUID_END_TOUCH_TRAMPOLINE
                        .store(patch.trampoline, std::sync::atomic::Ordering::SeqCst);
                    *FLUID_END_TOUCH
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(patch);
                    " + FluidEndTouch".to_string()
                }
                Err(why) => format!(" (removing a volume with props in it may crash: {why})"),
            };
            if let Ok(t) = engine_hook::scan_sig(base, engine_hook::DISPATCH_EFFECT_SIG) {
                engine_hook::DISPATCH_EFFECT_ADDR
                    .store(t as usize, std::sync::atomic::Ordering::SeqCst);
            }

            // Explosion events in a volume become the water explosion effect.
            let te_note = (|| -> Result<(), &'static str> {
                if engine_hook::DISPATCH_EFFECT_ADDR.load(std::sync::atomic::Ordering::SeqCst) == 0
                {
                    return Err("DispatchEffect not found");
                }
                let vtable = engine_hook::te_explosion_vtable(base)?;
                engine_hook::TE_EXPLOSION_VTABLE.store(vtable, std::sync::atomic::Ordering::SeqCst);
                let create = engine_hook::scan_sig(base, engine_hook::TE_CREATE_SIG)?;
                let patch = engine_hook::install_patch(
                    create,
                    engine_hook::TE_CREATE_SIG,
                    engine_hook::TE_CREATE_STOLEN,
                    engine_hook::hook_te_create as *const () as usize,
                    &engine_hook::win::PLATFORM,
                )?;
                engine_hook::ORIGINAL_TE_CREATE
                    .store(patch.trampoline, std::sync::atomic::Ordering::SeqCst);
                *TE_HOOK
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(patch);
                Ok(())
            })();
            let te_note = match te_note {
                Ok(()) => " + explosion events".to_string(),
                Err(why) => format!(" (explosion events not hooked: {why})"),
            };

            // The world entity, named as the hit entity of every water hit. If it
            // cannot be found, water hits that the engine gave no entity are
            // skipped rather than reported without one.
            let world_note = match engine_hook::entity_array_global(base) {
                Ok(g) => {
                    engine_hook::ENTITY_ARRAY_GLOBAL.store(g, std::sync::atomic::Ordering::SeqCst);
                    String::new()
                }
                Err(why) => format!(" (world entity unknown: {why})"),
            };

            // Fourth: explosives in a volume take the water explosion.
            let mut patches = Vec::new();
            for i in 0..engine_hook::EXPLOSION_TARGETS.len() {
                let r = engine_hook::find_explosion_target(base, i).and_then(|(t, sig, stolen)| {
                    engine_hook::install_patch(
                        t,
                        sig,
                        stolen,
                        engine_hook::EXPLOSION_HOOKS[i] as usize,
                        &engine_hook::win::PLATFORM,
                    )
                });
                if let Ok(patch) = r {
                    engine_hook::EXPLOSION_TRAMPOLINES[i]
                        .store(patch.trampoline, std::sync::atomic::Ordering::SeqCst);
                    patches.push(patch);
                }
            }
            let explosion = format!(
                " + {}/{} explosion checks",
                patches.len(),
                engine_hook::EXPLOSION_TARGETS.len()
            );
            *EXPLOSIONS
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = patches;
            Ok(format!(
                "{summary}{bubble}{end_touch}{explosion}{te_note}{world_note}"
            ))
        }
    }

    pub(crate) static EXPLOSIONS: Mutex<Vec<engine_hook::Patch>> = Mutex::new(Vec::new());
    pub(crate) static TE_HOOK: Mutex<Option<engine_hook::Patch>> = Mutex::new(None);

    pub(crate) static BUBBLE: Mutex<Option<engine_hook::Patch>> = Mutex::new(None);
    pub(crate) static FLUID_END_TOUCH: Mutex<Option<engine_hook::Patch>> = Mutex::new(None);

    /// (id, controller pointer, contents) for every fluid this module created.
    pub(crate) static FLUIDS: Mutex<Vec<(u32, usize, c_int)>> = Mutex::new(Vec::new());
    static NEXT_FLUID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

    unsafe fn physenv() -> Result<*mut c_void, &'static str> {
        unsafe {
            let base = engine_hook::win::server_base();
            if base.is_null() {
                return Err("server.dll is not loaded");
            }
            let at = engine_hook::scan_sig(base, engine_hook::PHYSENV_SIG)
                .map_err(|_| "physics environment pointer not found in server.dll")?;
            let env = engine_hook::physenv_global_from(at).read();
            if env.is_null() {
                return Err("the physics environment has not been created yet");
            }
            Ok(env)
        }
    }

    pub(crate) unsafe fn fluid_attach(
        object: usize,
        plane: [f32; 4],
        fluid: &FluidSettings,
    ) -> Result<u32, &'static str> {
        unsafe {
            let env = physenv()?;
            let params = engine_hook::FluidParams {
                surface_plane: plane,
                current_velocity: fluid.current,
                damping: fluid.damping,
                torque_factor: fluid.torque,
                viscosity_factor: fluid.viscosity,
                game_data: std::ptr::null_mut(),
                use_aerodynamics: u8::from(fluid.aerodynamics),
                contents: fluid.contents,
            };
            if object == 0 {
                return Err("null physics object");
            }
            engine_hook::add_fluid_callback_flag(object as *mut _);
            let controller = engine_hook::create_fluid(env, object as *mut _, &params);
            if controller.is_null() {
                return Err(
                    "the engine refused to create a fluid controller (object has no physics?)",
                );
            }
            let id = NEXT_FLUID.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            FLUIDS
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push((id, controller as usize, fluid.contents));
            engine_hook::fluid_born(controller as usize);
            Ok(id)
        }
    }

    pub(crate) unsafe fn fluid_detach(id: u32) -> bool {
        unsafe {
            let taken = {
                let mut list = FLUIDS
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                list.iter().position(|f| f.0 == id).map(|i| list.remove(i))
            };
            let Some((_, fluid, contents)) = taken else {
                return false;
            };
            let Ok(env) = physenv() else {
                return false;
            };
            engine_hook::destroy_fluid(env, fluid as *mut _);
            engine_hook::fluid_died(fluid, contents);
            true
        }
    }

    pub(crate) unsafe fn fluid_detach_all() {
        unsafe {
            let ids: Vec<u32> = FLUIDS
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .iter()
                .map(|f| f.0)
                .collect();
            for id in ids {
                fluid_detach(id);
            }
        }
    }

    pub(crate) fn fluid_count() -> usize {
        FLUIDS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }

    pub(crate) unsafe fn uninstall_contents() {
        unsafe {
            engine_hook::RAY_SWEPT_OFFSET.store(
                engine_hook::RAY_SWEPT_OFFSET_OLD,
                std::sync::atomic::Ordering::SeqCst,
            );
            engine_hook::DISPATCH_EFFECT_ADDR.store(0, std::sync::atomic::Ordering::SeqCst);
            let te_patch = TE_HOOK
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            if let Some(patch) = te_patch {
                engine_hook::uninstall_patch(&patch, &engine_hook::win::PLATFORM);
            }
            engine_hook::TE_EXPLOSION_VTABLE.store(0, std::sync::atomic::Ordering::SeqCst);
            let explosion_patches = std::mem::take(
                &mut *EXPLOSIONS
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            );
            for patch in explosion_patches {
                engine_hook::uninstall_patch(&patch, &engine_hook::win::PLATFORM);
            }
            let patch = FLUID_END_TOUCH
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            if let Some(patch) = patch {
                engine_hook::uninstall_patch(&patch, &engine_hook::win::PLATFORM);
            }
            engine_hook::forget_dead_fluids();
            // Bubble detour first: it calls through the trampoline, not the slots.
            let patch = BUBBLE
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            if let Some(patch) = patch {
                engine_hook::uninstall_patch(&patch, &engine_hook::win::PLATFORM);
            }
            let taken = std::mem::take(
                &mut *CONTENTS
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            );
            for (entry, original, hook) in taken.into_iter().rev() {
                engine_hook::uninstall_slot_hook(
                    entry,
                    hook,
                    original,
                    &engine_hook::win::PLATFORM,
                );
            }
        }
    }

    pub(crate) fn contents_active() -> bool {
        !CONTENTS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty()
    }

    /// Client realm: the same two hooks on the client's own `IEngineTrace`, so what the
    /// client does with water (ripples and splashes, the underwater sound filter,
    /// prediction) sees the volumes the Lua side gave it.
    pub(crate) unsafe fn install_client_contents() -> Result<String, &'static str> {
        unsafe {
            let base = engine_hook::win::client_base();
            if base.is_null() {
                return Err("client.dll is not loaded");
            }
            let at = engine_hook::scan_sig(base, engine_hook::CLIENT_TRACE_SIG)?;
            let trace = engine_hook::trace_global_from_load(at).read();
            let (slot, original) =
                engine_hook::install_contents_hook(trace, &engine_hook::win::PLATFORM)?;
            let mut hooks = vec![(
                slot,
                original,
                engine_hook::hook_get_point_contents as *const () as usize,
            )];
            // TraceRay needs the Ray_t layout and a world entity to name in the result;
            // without either only the point contents are hooked.
            let entities = engine_hook::win::client_interface(c"VClientEntityList003");
            let ray = engine_hook::scan_sig(base, engine_hook::RAY_HELPER_SIG);
            let note = match (ray, entities) {
                (Ok(f), e) if e != 0 => {
                    let off = engine_hook::ray_swept_offset_from(f);
                    engine_hook::RAY_SWEPT_OFFSET.store(off, std::sync::atomic::Ordering::SeqCst);
                    engine_hook::CLIENT_ENTITY_LIST.store(e, std::sync::atomic::Ordering::SeqCst);
                    let trace_hook = engine_hook::hook_trace_ray as *const () as usize;
                    match engine_hook::install_slot_hook(
                        trace,
                        engine_hook::TRACE_RAY_SLOT,
                        trace_hook,
                        &engine_hook::ORIGINAL_TRACE_RAY,
                        &engine_hook::win::PLATFORM,
                    ) {
                        Ok((entry, orig)) => {
                            hooks.push((entry, orig, trace_hook));
                            format!("GetPointContents + TraceRay (Ray_t m_IsSwept at +{off:#x})")
                        }
                        Err(why) => format!("GetPointContents only (TraceRay hook failed: {why})"),
                    }
                }
                (Err(why), _) => format!("GetPointContents only (Ray_t helper: {why})"),
                _ => "GetPointContents only (no VClientEntityList003)".to_string(),
            };
            *CONTENTS
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = hooks;
            Ok(note)
        }
    }
}

#[cfg(not(all(windows, target_arch = "x86_64")))]
mod hook_state {
    pub(crate) const unsafe fn install() -> Result<usize, &'static str> {
        Err("engine hook is only available for the Windows x64 server.dll")
    }
    pub(crate) const unsafe fn uninstall() {}
    pub(crate) const fn active() -> bool {
        false
    }
    pub(crate) const unsafe fn install_contents() -> Result<String, &'static str> {
        Err("engine hook is only available for the Windows x64 server.dll")
    }
    pub(crate) const unsafe fn uninstall_contents() {}
    pub(crate) const fn contents_active() -> bool {
        false
    }
    pub(crate) const unsafe fn fluid_attach(
        _: usize,
        _: [f32; 4],
        _: &super::FluidSettings,
    ) -> Result<u32, &'static str> {
        Err("fluids are only available for the Windows x64 server.dll")
    }
    pub(crate) const unsafe fn fluid_detach(_: u32) -> bool {
        false
    }
    pub(crate) const unsafe fn fluid_detach_all() {}
    pub(crate) const fn fluid_count() -> usize {
        0
    }
}

static HOOK_STATUS: Mutex<String> = Mutex::new(String::new());

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

/// waterlevel.Set(ent, level) / ent:SetWaterLevelOverride(level)
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

/// waterlevel.Get(ent) / `ent:GetWaterLevelOverride()`
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

/// waterlevel.Clear(ent) / `ent:ClearWaterLevelOverride()`
unsafe extern "C-unwind" fn l_clear(l: LuaState) -> c_int {
    unsafe {
        let a = api();
        let removed = ent_index(l, 1).is_some_and(|idx| overrides().remove(&idx).is_some());
        (a.pushboolean)(l, c_int::from(removed));
        1
    }
}

/// `waterlevel.ClearAll()`
unsafe extern "C-unwind" fn l_clear_all(_l: LuaState) -> c_int {
    overrides().clear();
    0
}

/// waterlevel.SetSaveKey(key) - "" disables the per-tick datamap write.
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

/// `waterlevel.IsHooked()`
unsafe extern "C-unwind" fn l_is_hooked(l: LuaState) -> c_int {
    unsafe {
        (api().pushboolean)(l, c_int::from(hook_state::active()));
        1
    }
}

/// `waterlevel.HookStatus()`
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

/// `waterlevel.HookHits()`
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

/// waterlevel.AddVolume(x1, y1, z1, x2, y2, z2 [, liquid]) -> id | nil, err
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

/// waterlevel.RemoveVolume(id) -> bool
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

/// `waterlevel.ClearVolumes()`
unsafe extern "C-unwind" fn l_clear_volumes(_l: LuaState) -> c_int {
    volumes().clear();
    0
}

/// `waterlevel.VolumeCount()`
unsafe extern "C-unwind" fn l_volume_count(l: LuaState) -> c_int {
    unsafe {
        let n = volumes().len();
        (api().pushnumber)(l, n as f64);
        1
    }
}

/// waterlevel.PointInVolume(x, y, z) -> bool (pure lookup, no engine needed)
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

/// `waterlevel.IsContentsHooked()`
unsafe extern "C-unwind" fn l_is_contents_hooked(l: LuaState) -> c_int {
    unsafe {
        (api().pushboolean)(l, c_int::from(hook_state::contents_active()));
        1
    }
}

/// `waterlevel.ContentsHits()` - engine point queries answered with water
unsafe extern "C-unwind" fn l_contents_hits(l: LuaState) -> c_int {
    unsafe {
        let n = VOLUME_HITS.load(std::sync::atomic::Ordering::Relaxed);
        (api().pushnumber)(l, n as f64);
        1
    }
}

/// `waterlevel.TraceHits()` - engine traces rewritten as water-surface hits
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

/// waterlevel.AttachFluid(physobj, nx, ny, nz, dist [, damping] [, options]) -> id | nil, err
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

/// waterlevel.DetachFluid(id) -> bool. Call before removing the entity.
unsafe extern "C-unwind" fn l_detach_fluid(l: LuaState) -> c_int {
    unsafe {
        let a = api();
        let ok =
            (a.type_)(l, 1) == LUA_TNUMBER && hook_state::fluid_detach((a.tonumber)(l, 1) as u32);
        (a.pushboolean)(l, c_int::from(ok));
        1
    }
}

/// `waterlevel.FluidCount()`
unsafe extern "C-unwind" fn l_fluid_count(l: LuaState) -> c_int {
    unsafe {
        (api().pushnumber)(l, hook_state::fluid_count() as f64);
        1
    }
}

/// `waterlevel.BubbleClips()` - bullet bubble trails clipped to a volume
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

/// `waterlevel.ExplosionFixes()` - explosives whose water level was raised
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

/// `waterlevel.ExplosionsRedirected()` - explosion events sent as the water explosion
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

unsafe fn set_nil(l: LuaState, table: c_int, name: &CStr) {
    unsafe {
        let a = api();
        (a.pushnil)(l);
        (a.setfield)(l, table, name.as_ptr());
    }
}

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
        let a = api();
        let base = (a.gettop)(l);

        // Loaded as gmcl_waterlevelercore: the client's water. The Lua side gives it the
        // volumes (AddVolume etc.); its engine trace is
        // patched so the client sees and hears them like the server does.
        #[cfg(all(windows, target_arch = "x86_64"))]
        if client_module::is_client_module() {
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
            set_fn(l, t, c"HookStatus", l_hook_status);
            (a.setfield)(l, LUA_GLOBALSINDEX, c"waterlevelercore".as_ptr());
            (a.settop)(l, base);
            return 0;
        }

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
        let water_level = match hook_state::install() {
            Ok(rva) => format!("hooked CBaseEntity::SetWaterLevel at server.dll+{rva:#x}"),
            Err(why) => format!("engine hook inactive: {why}"),
        };
        let contents = match hook_state::install_contents() {
            Ok(what) => format!("water volumes active via {what}"),
            Err(why) => format!("water volumes inactive: {why}"),
        };
        let status = format!("{water_level}; {contents}");
        lua_print(l, &format!("[waterlevel] {status}"));
        *HOOK_STATUS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = status;

        (a.settop)(l, base);
        0
    }
}

#[no_mangle]
pub unsafe extern "C-unwind" fn gmod13_close(l: LuaState) -> c_int {
    unsafe {
        if API.get().is_none() {
            return 0;
        }
        let a = api();
        let base = (a.gettop)(l);

        #[cfg(all(windows, target_arch = "x86_64"))]
        if client_module::is_client_module() {
            hook_state::uninstall_contents();
            engine_hook::CLIENT_ENTITY_LIST.store(0, std::sync::atomic::Ordering::SeqCst);
            volumes().clear();
            (a.pushnil)(l);
            (a.setfield)(l, LUA_GLOBALSINDEX, c"waterlevelercore".as_ptr());
            (a.settop)(l, base);
            return 0;
        }

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
// Tests (Linux x86-64): exercise the real hook machinery against the real
// bytes of SetWaterLevel taken from a 64-bit Windows server.dll.
//   SERVER_DLL_X64=/path/to/server.dll cargo test --release
// ---------------------------------------------------------------------------

#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
mod tests {
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

            let original = install(target, &TEST_PLATFORM).expect("install failed");
            assert_eq!(&original[..5], &[0x48, 0x89, 0x5C, 0x24, 0x08]);
            // The entry now starts with the detour.
            assert_eq!(std::slice::from_raw_parts(target, 2), &[0xFF, 0x25]);

            // Fake CBaseEntity: edict index 5, current m_nWaterLevel = 1.
            let mut edict = [0u8; 32];
            edict[6..8].copy_from_slice(&5i16.to_le_bytes());
            let mut entity = vec![0u8; 0x400];
            entity[OFF_PEV..OFF_PEV + 8].copy_from_slice(&(edict.as_ptr() as u64).to_le_bytes());
            entity[0x2ab] = 1;
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
            assert_eq!(entity[0x2ab], 1);
            assert_eq!(HITS.load(Ordering::Relaxed), hits_before + 1);

            // 2) No override: the call goes through the trampoline, which runs
            //    the relocated prologue and continues in the real function.
            //    level == current again so it returns without side effects.
            overrides().remove(&5);
            set_water_level(this, 1);
            assert_eq!(entity[0x2ab], 1);
            assert_eq!(HITS.load(Ordering::Relaxed), hits_before + 1);

            // 3) Entities without a pev are passed through untouched.
            let mut bare = vec![0u8; 0x400];
            bare[0x2ab] = 2;
            set_water_level(bare.as_mut_ptr().cast::<c_void>(), 2);
            assert_eq!(bare[0x2ab], 2);

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
            let t =
                std::slice::from_raw_parts(patch.trampoline as *const u8, BUBBLE_TRAIL_STOLEN + 14);
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
            let at =
                scan_sig(img.as_ptr(), PHYSENV_SIG).expect("signature must match exactly once");
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
        FLUID_CALLS.lock().unwrap().push((
            "destroy",
            env as usize,
            fluid as usize,
            [0.0; 4],
            0.0,
            0,
        ));
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
        let put =
            |ray: &mut Vec<u8>, o: usize, v: f32| ray[o..o + 4].copy_from_slice(&v.to_le_bytes());
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
            let f =
                scan_sig(img.as_ptr(), RAY_HELPER_SIG).expect("ray helper must match exactly once");
            let off = ray_swept_offset_from(f);
            assert!(
                off == 0x41 || off == 0x49,
                "unexpected m_IsSwept offset {off:#x}"
            );
        }
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
            let t = 172.0 / 300.0;
            assert!((f32at(&tr, 0x2c) - t).abs() < 1e-6, "fraction");
            assert!(
                (f32at(&tr, 0x0c + 8) - 128.0).abs() < 1e-3,
                "endpos.z is the surface"
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
            assert!((f32at(&tr, 0x2c) - 172.0 / 300.0).abs() < 1e-6);
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
}
