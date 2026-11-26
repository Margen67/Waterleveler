// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67

use super::*;

/// A buffer whose every byte is distinct, so a changed or lost byte shows.
fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8 + 1).collect()
}

#[test]
pub(crate) fn trace_head_round_trip_keeps_every_byte() {
    let original = pattern(0x70);
    let mut buf = original.clone();
    unsafe {
        let head: TraceHead = read_struct(buf.as_ptr());
        write_struct(buf.as_mut_ptr(), &head);
    }
    assert_eq!(buf, original, "padding and unused fields must survive");
}

#[test]
pub(crate) fn trace_head_changes_only_the_field_that_was_set() {
    let original = pattern(0x70);
    let mut buf = original.clone();
    unsafe {
        let mut head: TraceHead = read_struct(buf.as_ptr());
        head.fraction = 0.25;
        head.start_solid = 0;
        write_struct(buf.as_mut_ptr(), &head);
    }
    let mut expected = original;
    expected[0x2c..0x30].copy_from_slice(&0.25f32.to_le_bytes());
    expected[0x37] = 0;
    assert_eq!(buf, expected);
}

#[test]
pub(crate) fn ray_head_reads_from_an_unaligned_address() {
    let mut buf = [0u8; 0x50];
    let floats: [(usize, f32); 12] = [
        (0x00, 1.0),
        (0x04, 2.0),
        (0x08, 3.0),
        (0x10, 4.0),
        (0x14, 5.0),
        (0x18, 6.0),
        (0x20, 7.0),
        (0x24, 8.0),
        (0x28, 9.0),
        (0x30, 10.0),
        (0x34, 11.0),
        (0x38, 12.0),
    ];
    for (at, v) in floats {
        buf[at + 1..at + 5].copy_from_slice(&v.to_le_bytes());
    }
    let ray: RayHead = unsafe { read_struct(buf.as_ptr().add(1)) };
    assert_eq!(ray.start, [1.0, 2.0, 3.0]);
    assert_eq!(ray.delta, [4.0, 5.0, 6.0]);
    assert_eq!(ray.offset, [7.0, 8.0, 9.0]);
    assert_eq!(ray.extents, [10.0, 11.0, 12.0]);
}

#[test]
pub(crate) fn read_vec3_needs_no_alignment() {
    let mut buf = [0u8; 16];
    for (i, v) in [1.5f32, -2.5, 3.5].into_iter().enumerate() {
        buf[1 + i * 4..5 + i * 4].copy_from_slice(&v.to_le_bytes());
    }
    let v = unsafe { read_vec3(buf.as_ptr().add(1).cast::<f32>()) };
    assert_eq!(v, [1.5, -2.5, 3.5]);
}

#[test]
pub(crate) fn effect_data_is_the_bytes_the_engine_expects() {
    let d: [u8; 0xa0] = zerocopy::transmute!(EffectData::new([1.0, 2.0, 3.0], 0x428, 128.0, 50.0));
    let mut expected = [0u8; 0xa0];
    expected[0..4].copy_from_slice(&1.0f32.to_le_bytes());
    expected[4..8].copy_from_slice(&2.0f32.to_le_bytes());
    expected[8..12].copy_from_slice(&3.0f32.to_le_bytes());
    expected[0x30..0x34].copy_from_slice(&0x428u32.to_le_bytes());
    expected[0x38..0x3c].copy_from_slice(&128.0f32.to_le_bytes());
    expected[0x3c..0x40].copy_from_slice(&50.0f32.to_le_bytes());
    expected[0x88] = 1;
    assert_eq!(d, expected);
}
