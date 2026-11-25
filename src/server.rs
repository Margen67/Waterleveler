// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67
// Server realm (gmsv_waterlevelercore).

mod bubble;
mod entity;
mod explosion;
mod fluid;
// The engine's constants and layouts, per platform: Windows (also under `cfg(test)`) or Linux.
#[cfg(not(any(windows, test)))]
mod linux;
mod lua_api;
mod patch;
mod state;
#[cfg(any(windows, test))]
mod windows;

pub(crate) use bubble::*;
pub(crate) use entity::*;
pub(crate) use explosion::*;
pub(crate) use fluid::*;
#[cfg(not(any(windows, test)))]
pub(crate) use linux::*;
pub(crate) use lua_api::*;
pub(crate) use patch::*;
pub(crate) use state::*;
#[cfg(any(windows, test))]
pub(crate) use windows::*;
