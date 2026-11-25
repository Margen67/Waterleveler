// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67
// Code shared by the server and the client module.

#[macro_use]
mod abi;
pub(crate) mod hook_state;
mod image;
#[cfg(target_os = "linux")]
mod linux;
mod lua_api;
mod memory;
#[cfg(all(feature = "server", feature = "client"))]
mod realm;
mod scan;
mod state;
mod trace;
#[cfg(windows)]
mod windows;

pub(crate) use image::*;
#[cfg(target_os = "linux")]
pub(crate) use linux::*;
pub(crate) use lua_api::*;
pub(crate) use memory::*;
#[cfg(all(feature = "server", feature = "client"))]
pub(crate) use realm::*;
pub(crate) use scan::*;
pub(crate) use state::*;
pub(crate) use trace::*;
#[cfg(windows)]
pub(crate) use windows::*;
