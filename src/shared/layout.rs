// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67

//! The engine structures this module reads and writes, as plain `#[repr(C)]` types, so the
//! fields are named instead of reached with pointer arithmetic. The offsets come from the
//! disassembly (see `shared/trace.rs`); the asserts below fail the build if a struct and
//! its documented offsets ever disagree. `zerocopy` proves that the types have no hidden
//! padding and that any bytes are a valid value, but it cannot know what the engine's own
//! layout is: that is what the offsets are for.
//!
//! Anything the engine owns that this module does not use stays in explicit `_pad` fields,
//! so reading a struct, changing a field and writing it back leaves the rest as it was.

// The `FromBytes` derive generates empty enums to check the field types, which newer
// nightly clippy reports (`empty_enums`) at the derive; the code is not ours.
#![allow(clippy::empty_enums)]

use std::mem::{offset_of, size_of};
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout};

/// Reads a `T` from `at`, which needs no alignment.
///
/// # Safety
/// `at` must be valid for reads of `size_of::<T>()` bytes.
pub(crate) unsafe fn read_struct<T: FromBytes + KnownLayout + Immutable>(at: *const u8) -> T {
    // SAFETY: the caller guarantees the bytes are readable.
    let bytes = unsafe { std::slice::from_raw_parts(at, size_of::<T>()) };
    // The slice is exactly `T`'s size, so this cannot fail; the fallback only avoids a panic.
    T::read_from_bytes(bytes).unwrap_or_else(|_| T::new_zeroed())
}

/// Writes `value` to `at`, which needs no alignment.
///
/// # Safety
/// `at` must be valid for writes of `size_of::<T>()` bytes.
pub(crate) unsafe fn write_struct<T: IntoBytes + Immutable>(at: *mut u8, value: &T) {
    let bytes = value.as_bytes();
    // SAFETY: the caller guarantees the destination; the source is a separate local value.
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), at, bytes.len()) };
}

/// A `Vector` the engine passes by pointer.
///
/// # Safety
/// `at` must be valid for reads of three floats.
pub(crate) unsafe fn read_vec3(at: *const f32) -> [f32; 3] {
    unsafe { read_struct(at.cast::<u8>()) }
}

/// The first 64 bytes of `Ray_t`: four `Vector`s, each padded to 16 bytes. What follows
/// (the flags) sits at an offset that depends on the game build.
#[derive(Clone, Copy, Debug, FromBytes, KnownLayout, Immutable)]
#[repr(C)]
pub(crate) struct RayHead {
    pub start: [f32; 3],
    _pad_start: f32,
    pub delta: [f32; 3],
    _pad_delta: f32,
    pub offset: [f32; 3],
    _pad_offset: f32,
    pub extents: [f32; 3],
    _pad_extents: f32,
}

const _: () = {
    assert!(size_of::<RayHead>() == 0x40);
    assert!(offset_of!(RayHead, start) == 0x00);
    assert!(offset_of!(RayHead, delta) == 0x10);
    assert!(offset_of!(RayHead, offset) == 0x20);
    assert!(offset_of!(RayHead, extents) == 0x30);
};

/// `trace_t` (`CGameTrace`) up to and including `hitbox`.
#[derive(Clone, Copy, Debug, FromBytes, IntoBytes, KnownLayout, Immutable)]
#[repr(C)]
pub(crate) struct TraceHead {
    pub start_pos: [f32; 3],
    pub end_pos: [f32; 3],
    pub plane_normal: [f32; 3],
    pub plane_dist: f32,
    pub plane_type: u8,
    pub plane_sign_bits: u8,
    _pad_plane: [u8; 2],
    pub fraction: f32,
    pub contents: i32,
    pub disp_flags: u16,
    pub all_solid: u8,
    pub start_solid: u8,
    pub fraction_left_solid: f32,
    _pad_left_solid: [u8; 4],
    /// `surface.name`, a pointer.
    pub surface_name: u64,
    pub surface_props: i16,
    pub surface_flags: u16,
    _pad_surface: [u8; 4],
    pub hit_group: i32,
    pub physics_bone: i16,
    _pad_bone: [u8; 2],
    /// `m_pEnt`, a pointer.
    pub ent: u64,
    pub hitbox: i32,
    _pad_tail: [u8; 4],
}

const _: () = {
    assert!(size_of::<TraceHead>() == 0x68);
    assert!(offset_of!(TraceHead, start_pos) == 0x00);
    assert!(offset_of!(TraceHead, end_pos) == 0x0c);
    assert!(offset_of!(TraceHead, plane_normal) == 0x18);
    assert!(offset_of!(TraceHead, plane_dist) == 0x24);
    assert!(offset_of!(TraceHead, plane_type) == 0x28);
    assert!(offset_of!(TraceHead, plane_sign_bits) == 0x29);
    assert!(offset_of!(TraceHead, fraction) == 0x2c);
    assert!(offset_of!(TraceHead, contents) == 0x30);
    assert!(offset_of!(TraceHead, disp_flags) == 0x34);
    assert!(offset_of!(TraceHead, all_solid) == 0x36);
    assert!(offset_of!(TraceHead, start_solid) == 0x37);
    assert!(offset_of!(TraceHead, fraction_left_solid) == 0x38);
    assert!(offset_of!(TraceHead, surface_name) == 0x40);
    assert!(offset_of!(TraceHead, surface_props) == 0x48);
    assert!(offset_of!(TraceHead, surface_flags) == 0x4a);
    assert!(offset_of!(TraceHead, hit_group) == 0x50);
    assert!(offset_of!(TraceHead, physics_bone) == 0x54);
    assert!(offset_of!(TraceHead, ent) == 0x58);
    assert!(offset_of!(TraceHead, hitbox) == 0x60);
};

/// `CEffectData`, as handed to `DispatchEffect` (only the server builds one).
#[cfg(feature = "server")]
#[derive(Clone, Copy, Debug, FromBytes, IntoBytes, KnownLayout, Immutable)]
#[repr(C)]
pub(crate) struct EffectData {
    pub origin: [f32; 3],
    _pad_vectors: [u8; 0x24],
    pub flags: u32,
    _pad_entity: [u8; 4],
    pub scale: f32,
    pub magnitude: f32,
    _pad_misc: [u8; 0x48],
    /// The byte the engine sets for the water explosion.
    pub flag_88: u8,
    _pad_tail: [u8; 0x17],
}

#[cfg(feature = "server")]
const _: () = {
    assert!(size_of::<EffectData>() == 0xa0);
    assert!(offset_of!(EffectData, origin) == 0x00);
    assert!(offset_of!(EffectData, flags) == 0x30);
    assert!(offset_of!(EffectData, scale) == 0x38);
    assert!(offset_of!(EffectData, magnitude) == 0x3c);
    assert!(offset_of!(EffectData, flag_88) == 0x88);
};

#[cfg(feature = "server")]
impl EffectData {
    pub(crate) fn new(origin: [f32; 3], flags: u32, scale: f32, magnitude: f32) -> Self {
        Self {
            origin,
            flags,
            scale,
            magnitude,
            flag_88: 1,
            ..<Self as zerocopy::FromZeros>::new_zeroed()
        }
    }
}
