// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67
// Code shared by the server and the client module.

#[cfg(target_os = "linux")]
mod linux;
mod lua;
mod lua_api;
mod state;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "linux")]
pub(crate) use linux::*;
pub(crate) use lua::*;
pub(crate) use lua_api::*;
pub(crate) use state::*;
#[cfg(windows)]
pub(crate) use windows::*;
