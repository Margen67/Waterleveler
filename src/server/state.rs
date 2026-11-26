// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67

use crate::*;
use std::ffi::CString;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::Mutex;

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// `None` = default key, `Some("")` = engine writes disabled.
pub(crate) static SAVE_KEY: Mutex<Option<String>> = Mutex::new(None);

/// The settings of a fluid controller (`fluidparams_t`) besides its surface plane.
///
/// A map's water brush has torque, viscosity and aerodynamics at 0 (its fluid
/// data only carries the plane, current, damping and contents), so those are the
/// defaults; damping is 0.01.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FluidSettings {
    pub damping: f32,
    pub current: [f32; 3],
    pub torque: f32,
    pub viscosity: f32,
    pub aerodynamics: bool,
    /// `Contents::WATER` or `Contents::SLIME`.
    pub contents: Contents,
}

impl Default for FluidSettings {
    fn default() -> Self {
        Self {
            damping: 0.01,
            current: [0.0; 3],
            torque: 0.0,
            viscosity: 0.0,
            aerodynamics: false,
            contents: Contents::WATER,
        }
    }
}

/// How many entity indices the override table covers: the 14 bits of an entity handle's index
/// (Windows edict indices are far below that).
pub(crate) const MAX_ENTITIES: usize = 1 << 14;

/// Marks an entity without an override.
const NO_OVERRIDE: u8 = u8::MAX;

/// Entity index -> overridden water level (0..=3) or `NO_OVERRIDE`. A dense table of atomics,
/// so the water-level hook (called for every entity) never takes a lock.
static LEVELS: [AtomicU8; MAX_ENTITIES] = [const { AtomicU8::new(NO_OVERRIDE) }; MAX_ENTITIES];

/// How many entities have an override, so the tick hook can skip scanning the table when
/// there are none.
pub(crate) static OVERRIDE_COUNT: AtomicUsize = AtomicUsize::new(0);

fn slot(idx: i32) -> Option<&'static AtomicU8> {
    usize::try_from(idx).ok().and_then(|i| LEVELS.get(i))
}

/// Overrides `idx`'s water level. Returns false if the index is outside the table.
pub(crate) fn override_set(idx: i32, level: u8) -> bool {
    let Some(slot) = slot(idx) else {
        return false;
    };
    if slot.swap(level, Ordering::Relaxed) == NO_OVERRIDE {
        OVERRIDE_COUNT.fetch_add(1, Ordering::Relaxed);
    }
    true
}

pub(crate) fn override_get(idx: i32) -> Option<u8> {
    slot(idx)
        .map(|s| s.load(Ordering::Relaxed))
        .filter(|&l| l != NO_OVERRIDE)
}

/// Returns whether there was an override to remove.
pub(crate) fn override_remove(idx: i32) -> bool {
    let removed = slot(idx).is_some_and(|s| s.swap(NO_OVERRIDE, Ordering::Relaxed) != NO_OVERRIDE);
    if removed {
        OVERRIDE_COUNT.fetch_sub(1, Ordering::Relaxed);
    }
    removed
}

pub(crate) fn override_clear() {
    for idx in 0..MAX_ENTITIES {
        if OVERRIDE_COUNT.load(Ordering::Relaxed) == 0 {
            break;
        }
        override_remove(idx as i32);
    }
}

/// Every override, in entity order.
pub(crate) fn override_entries() -> Vec<(i32, u8)> {
    if OVERRIDE_COUNT.load(Ordering::Relaxed) == 0 {
        return Vec::new();
    }
    (0..MAX_ENTITIES as i32)
        .filter_map(|idx| override_get(idx).map(|level| (idx, level)))
        .collect()
}

pub(crate) fn save_key() -> Option<CString> {
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
