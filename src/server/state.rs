// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67

use crate::*;
use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::Mutex;

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// Entity index -> overridden water level (0..=3).
pub(crate) static OVERRIDES: Mutex<BTreeMap<i32, u8>> = Mutex::new(BTreeMap::new());

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
pub(crate) static OVERRIDE_COUNT: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// The overrides map, locked. When it is released the entry count is published.
pub(crate) struct OverridesGuard(pub(crate) std::sync::MutexGuard<'static, BTreeMap<i32, u8>>);

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

pub(crate) fn overrides() -> OverridesGuard {
    OverridesGuard(
        OVERRIDES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    )
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
