// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67

use crate::*;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

pub(crate) static BUBBLE_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);

pub(crate) static BUBBLE_CLIPS: AtomicU64 = AtomicU64::new(0);

/// Parameter interval `[t0, t1]` (clamped to 0..1) in which the segment
/// `s + t*d` lies inside the box. `None` if it never does.
pub(crate) fn clip_segment_to_box(
    s: [f32; 3],
    d: [f32; 3],
    lo: [f32; 3],
    hi: [f32; 3],
) -> Option<(f32, f32)> {
    let mut t0 = 0.0f32;
    let mut t1 = 1.0f32;
    for i in 0..3 {
        if d[i].abs() < 1e-9 {
            if s[i] < lo[i] || s[i] > hi[i] {
                return None;
            }
            continue;
        }
        let a = (lo[i] - s[i]) / d[i];
        let b = (hi[i] - s[i]) / d[i];
        let (near, far) = if a < b { (a, b) } else { (b, a) };
        t0 = t0.max(near);
        t1 = t1.min(far);
        if t0 > t1 {
            return None;
        }
    }
    Some((t0, t1))
}

#[derive(Debug, PartialEq)]
pub(crate) enum BubbleClip {
    /// Call the engine function unchanged.
    Keep,
    /// Nothing worth drawing: skip the call.
    Skip,
    /// Call it with these end points and the count scaled by the fraction.
    Clipped([f32; 3], [f32; 3], f32),
}

pub(crate) fn clip_bubble_trail(vols: &[Volume], from: [f32; 3], to: [f32; 3]) -> BubbleClip {
    let d = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
    let len = d[2].mul_add(d[2], d[1].mul_add(d[1], d[0] * d[0])).sqrt();
    if len.is_nan() || len <= 1e-3 {
        return BubbleClip::Keep;
    }
    // A volume the trail starts in wins; otherwise the first one it enters.
    let mut best: Option<(f32, f32)> = None;
    let mut starts_inside = false;
    for v in vols {
        let Some((t0, t1)) = clip_segment_to_box(from, d, v.min, v.max) else {
            continue;
        };
        if t0 <= 0.0 {
            if !starts_inside || t1 > best.map_or(0.0, |b| b.1) {
                best = Some((t0, t1));
                starts_inside = true;
            }
        } else if !starts_inside && best.is_none_or(|b| t0 < b.0) {
            best = Some((t0, t1));
        }
    }
    let Some((mut a, mut b)) = best else {
        return BubbleClip::Keep; // not in a volume: the engine's own rules apply
    };
    if a <= 0.0 && b >= 1.0 {
        return BubbleClip::Keep; // entirely inside
    }
    // Keep the new end points a hair inside so the engine's own depth
    // probe there still sees water.
    let eps = 0.5 / len;
    if a > 0.0 {
        a += eps;
    }
    if b < 1.0 {
        b -= eps;
    }
    if b <= a {
        return BubbleClip::Skip;
    }
    let at = |t: f32| {
        [
            d[0].mul_add(t, from[0]),
            d[1].mul_add(t, from[1]),
            d[2].mul_add(t, from[2]),
        ]
    };
    BubbleClip::Clipped(at(a), at(b), b - a)
}

engine_abi_item! {
/// Replacement for `UTIL_BubbleTrail` (the engine's calling convention).
pub(crate) unsafe fn hook_bubble_trail(
    from: *const f32,
    to: *const f32,
    count: c_int,
) {
    unsafe {
        let original: engine_fn!((*const f32, *const f32, c_int)) =
            std::mem::transmute(BUBBLE_TRAMPOLINE.load(Ordering::Relaxed));
        let mut action = BubbleClip::Keep;
        if !from.is_null() && !to.is_null() {
            let f = [
                from.read_unaligned(),
                from.add(1).read_unaligned(),
                from.add(2).read_unaligned(),
            ];
            let t = [
                to.read_unaligned(),
                to.add(1).read_unaligned(),
                to.add(2).read_unaligned(),
            ];
            // A trail that stays clear of every volume is kept as it is, without a copy
            // of the snapshot. No lock is held when the original is called (it calls
            // back into the point-contents hook).
            let d = [t[0] - f[0], t[1] - f[1], t[2] - f[2]];
            if let Some(vols) =
                snapshot_if(|v| !segment_misses_bounds(v.bounds, f, d, [0.0; 3]))
            {
                action = clip_bubble_trail(&vols, f, t);
            }
        }
        match action {
            BubbleClip::Keep => original(from, to, count),
            BubbleClip::Skip => {
                bump(&BUBBLE_CLIPS);
            }
            BubbleClip::Clipped(a, b, frac) => {
                bump(&BUBBLE_CLIPS);
                let n = if count > 0 {
                    ((count as f32 * frac).ceil() as c_int).max(1)
                } else {
                    count
                };
                original(a.as_ptr(), b.as_ptr(), n);
            }
        }
    }
}
}
