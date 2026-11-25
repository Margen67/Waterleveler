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

// Crate-wide prelude: the std types the engine-ABI code in every file uses. Which of them a
// given build needs depends on the features and the platform, so unused ones are expected.
#[allow(unused_imports)]
pub(crate) use std::ffi::{c_char, c_int, c_void, CStr, CString};
#[allow(unused_imports)]
pub(crate) use std::sync::{Arc, Mutex};

#[macro_use]
mod shared;
#[cfg(feature = "client")]
mod client;
#[cfg(feature = "server")]
mod server;
#[cfg(feature = "client")]
pub(crate) use client::*;
#[cfg(feature = "server")]
pub(crate) use server::*;
pub(crate) use shared::*;

#[cfg(not(any(target_os = "linux", windows)))]
compile_error!("waterlevelercore supports Linux and Windows only");

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
mod test;
