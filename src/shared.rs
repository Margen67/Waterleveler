// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67
// Code shared by the server and the client module.

#[macro_use]
mod abi;
pub(crate) mod hook_state;
#[cfg(target_os = "linux")]
mod linux;
mod lua_api;
#[cfg(all(feature = "server", feature = "client"))]
mod realm;
mod scan;
mod state;
mod trace;
// The Windows code also builds under `cfg(test)`: the tests run a Windows server.dll's
// machine code on whatever host runs them.
#[cfg(any(windows, test))]
mod windows;

#[cfg(target_os = "linux")]
pub(crate) use linux::*;
pub(crate) use lua_api::*;
#[cfg(all(feature = "server", feature = "client"))]
pub(crate) use realm::*;
pub(crate) use scan::*;
pub(crate) use state::*;
pub(crate) use trace::*;
#[cfg(any(windows, test))]
pub(crate) use windows::*;
