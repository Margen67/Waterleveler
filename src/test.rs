// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67
// Exact float comparisons are on values the code copies or sets, items sit next to the
// test that uses them.
#![allow(clippy::float_cmp, clippy::items_after_statements)]
use super::*;

mod bubble;
mod explosion;
mod fluid;
mod layout;
mod linux;
mod signatures;
mod trace;
mod windows;

pub(crate) use linux::*;
pub(crate) use signatures::*;
pub(crate) use trace::*;
pub(crate) use windows::*;
