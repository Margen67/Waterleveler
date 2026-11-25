// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67

use crate::*;
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

/// The handful of OS services needed to change code protection.
pub(crate) struct Platform {
    /// Makes the range writable; returns the previous protection.
    pub make_writable: unsafe fn(*mut u8, usize) -> Option<u32>,
    pub restore_protect: unsafe fn(*mut u8, usize, u32),
}

pub(crate) static ORIGINAL_GPC: AtomicUsize = AtomicUsize::new(0);

engine_abi_item! {
/// Replacement for `IEngineTrace::GetPointContents` (vtable slot 0). All
/// four register arguments are forwarded untouched, so it does not matter
/// whether the engine's signature has a mask parameter or not. A point inside a volume
/// also reports its contents; so does the end point of the last small-hull trace, when
/// the bullet's own line reaches the surface it stopped at inside a volume.
pub(crate) unsafe fn hook_get_point_contents(
    this: *mut c_void,
    pos: *const f32,
    a3: usize,
    a4: usize,
) -> c_int {
    unsafe {
        let original: engine_fn!((*mut c_void, *const f32, usize, usize) -> c_int) =
            std::mem::transmute(ORIGINAL_GPC.load(Ordering::Relaxed));
        let mut contents = original(this, pos, a3, a4);
        if !pos.is_null() {
            // Never block inside an engine callback.
            let p = [pos.read(), pos.add(1).read(), pos.add(2).read()];
            let mut inside =
                snapshot_if(|v| v.may_contain(p)).map_or(0, |vols| vols.contents_at(p));
            if inside == 0 {
                // The end point of a small-hull trace (a shotgun pellet): the engine asks
                // whether it is in water to decide on a splash. Answer for where the
                // bullet's own line reaches the surface instead of the hull's centre.
                if let Some((end, probe)) = LAST_HULL.with(Cell::get) {
                    // The very floats the trace returned, so an exact comparison.
                    #[allow(clippy::float_cmp)]
                    let same = end == p;
                    if same {
                        if let Some(vols) = snapshot_if(|v| v.may_contain(probe)) {
                            inside = vols.contents_at(probe);
                        }
                        if DEBUG.load(Ordering::Relaxed) {
                            debug_log(format!(
                                "hull pellet end ({:.4}, {:.4}, {:.4}): its line reaches the surface at ({:.4}, {:.4}, {:.4}) -> {}",
                                p[0], p[1], p[2], probe[0], probe[1], probe[2],
                                if inside != 0 { "water" } else { "not water" }
                            ));
                        }
                    }
                }
            }
            if inside != 0 {
                contents |= inside;
                bump(&VOLUME_HITS);
            }
            if inside == 0 && DEBUG.load(Ordering::Relaxed) {
                // A point just outside every volume: a bullet's impact point that lands
                // here is not "in the water", so it gets no splash.
                if let Some(vols) = snapshot_if(|v| v.near(p, 2.0)) {
                    if vols.contents_at(p) == 0 {
                        debug_log(format!(
                            "point ({:.4}, {:.4}, {:.4}) is just outside a volume: not water",
                            p[0], p[1], p[2]
                        ));
                    }
                }
            }
        }
        contents
    }
}
}

/// Swaps vtable slot 0 of the `IEngineTrace` object. No machine code is
/// patched. Returns (slot address, original function pointer).
pub(crate) unsafe fn install_contents_hook(
    trace: *mut c_void,
    p: &Platform,
) -> Result<(usize, usize), &'static str> {
    unsafe {
        if trace.is_null() {
            return Err("enginetrace pointer is null");
        }
        let vtable = (trace as *const *mut usize).read();
        if vtable.is_null() {
            return Err("enginetrace has no vtable");
        }
        let original = vtable.read();
        let hook = hook_get_point_contents as *const () as usize;
        if original == 0 || original == hook {
            return Err("GetPointContents slot is empty or already hooked");
        }
        ORIGINAL_GPC.store(original, Ordering::SeqCst);
        let Some(old) = (p.make_writable)(vtable.cast::<u8>(), 8) else {
            return Err("could not make the vtable writable");
        };
        vtable.write(hook);
        (p.restore_protect)(vtable.cast::<u8>(), 8, old);
        Ok((vtable as usize, original))
    }
}

#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
pub(crate) unsafe fn uninstall_contents_hook(slot: usize, original: usize, p: &Platform) {
    unsafe {
        let slot = slot as *mut usize;
        if slot.read() != hook_get_point_contents as *const () as usize {
            return; // somebody else replaced it after us; leave it alone
        }
        if let Some(old) = (p.make_writable)(slot.cast::<u8>(), 8) {
            slot.write(original);
            (p.restore_protect)(slot.cast::<u8>(), 8, old);
        }
    }
}

// ---- Water volumes vs. traces: IEngineTrace::TraceRay (vtable slot 4) --
//
// Verified against 130 TraceRay call sites in the 64-bit server.dll:
//   TraceRay(this, const Ray_t*, unsigned mask, ITraceFilter*, trace_t*)
//   Ray_t:   m_Start +0x00, m_Delta +0x10, m_StartOffset +0x20,
//            m_Extents +0x30 (3 floats each), m_IsRay +0x40, m_IsSwept +0x41
//   trace_t: startpos +0x00, endpos +0x0c, plane{normal +0x18, dist +0x24,
//            type +0x28, signbits +0x29}, fraction +0x2c, contents +0x30,
//            dispFlags +0x34 (u16), allsolid +0x36, startsolid +0x37,
//            fractionleftsolid +0x38, surface{name +0x40, props +0x48,
//            flags +0x4a}, hitgroup +0x50, physicsbone +0x54, m_pEnt +0x58,
//            hitbox +0x60

pub(crate) const TRACE_RAY_SLOT: usize = 4;

pub(crate) static ORIGINAL_TRACE_RAY: AtomicUsize = AtomicUsize::new(0);

pub(crate) static TRACE_HITS: AtomicU64 = AtomicU64::new(0);

/// While set, the hooks log what they answered near a volume (`waterleveler_debug 1`).
pub(crate) static DEBUG: AtomicBool = AtomicBool::new(false);

pub(crate) static DEBUG_LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Adds a line to the debug log (kept to the latest 512; never blocks).
pub(crate) fn debug_log(line: String) {
    if let Ok(mut log) = DEBUG_LOG.try_lock() {
        if log.len() < 512 {
            log.push(line);
        }
    }
}

/// The debug log so far, one line per entry; empties it.
pub(crate) fn take_debug_log() -> String {
    let mut log = DEBUG_LOG
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let text = log.join("\n");
    log.clear();
    text
}

/// Offset of `Ray_t::m_IsSwept`. The older 64-bit builds have it at +0x41
/// (`m_IsRay` +0x40); the x86-64 branch inserted an 8-byte member (the world
/// axis transform pointer) at +0x40, so the flags sit at +0x48/+0x49.
pub(crate) const RAY_SWEPT_OFFSET_OLD: usize = 0x41;

pub(crate) static RAY_SWEPT_OFFSET: AtomicUsize = AtomicUsize::new(RAY_SWEPT_OFFSET_OLD);

/// The engine helper that builds a `Ray_t` on its stack from two points
/// (`UTIL_TraceLine`-style): its `mov byte [rax+disp],1` stores `m_IsSwept`.
/// The frame is `mov rax,rsp; push rbx; sub rsp,0x90` with the `Ray_t` at
/// rsp+0x40, so offset = disp + 0x98 - 0x40. Byte 53 is disp.
pub(crate) const RAY_HELPER_SIG: &str =
    "48 8B C4 53 48 81 EC 90 00 00 00 F3 0F 10 42 08 F3 0F 10 19 \
    F3 0F 10 0A F3 0F 10 61 04 F3 0F 5C CB F3 0F 10 52 04 F3 0F 10 69 08 F3 0F 5C D4 F3 0F 5C C5 \
    C6 40 ?? 01";

pub(crate) const RAY_HELPER_DISP_BYTE: usize = 53;

pub(crate) const unsafe fn ray_swept_offset_from(func: *const u8) -> usize {
    unsafe {
        let disp = func.add(RAY_HELPER_DISP_BYTE).cast::<i8>().read() as isize;
        (disp + 0x98 - 0x40) as usize
    }
}

/// How far inside a volume, along the axis of the face it entered through, a rewritten
/// trace ends. More than the engine's 1/32 trace epsilon and its 0.1 downward probe
/// can cancel out.
pub(crate) const ENTRY_DEPTH: f32 = 0.125;

/// The surface name a rewritten trace reports, as a water brush's is.
pub(crate) static WATER_SURFACE_NAME: &[u8] = b"water\0";

/// Where the segment `s + t*d` (t in 0..1) first enters the box `[lo, hi]`
/// from outside: `(t, axis, entered_through_min_face)`. `None` if it never
/// does, or if the start is already inside or on the boundary.
pub(crate) fn segment_enters_box(
    s: [f32; 3],
    d: [f32; 3],
    lo: [f32; 3],
    hi: [f32; 3],
) -> Option<(f32, usize, bool)> {
    let mut tmin = f32::NEG_INFINITY;
    let mut tmax = f32::INFINITY;
    let mut axis = 0usize;
    let mut min_face = true;
    for i in 0..3 {
        if d[i].abs() < 1e-9 {
            if s[i] < lo[i] || s[i] > hi[i] {
                return None;
            }
            continue;
        }
        let t1 = (lo[i] - s[i]) / d[i];
        let t2 = (hi[i] - s[i]) / d[i];
        let (near, far, through_min) = if t1 < t2 {
            (t1, t2, true)
        } else {
            (t2, t1, false)
        };
        if near > tmin {
            tmin = near;
            axis = i;
            min_face = through_min;
        }
        tmax = tmax.min(far);
        if tmin > tmax {
            return None;
        }
    }
    if tmin.is_finite() && tmin > 0.0 && tmin < 1.0 {
        Some((tmin, axis, min_face))
    } else {
        None
    }
}

/// Where the segment `s + t*d` (t in 0..=1) leaves the box `[lo, hi]` that it
/// starts in: the exit fraction, or 1.0 if it stays inside.
pub(crate) fn segment_exit_fraction(s: [f32; 3], d: [f32; 3], lo: [f32; 3], hi: [f32; 3]) -> f32 {
    let mut exit = 1.0f32;
    for i in 0..3 {
        if d[i].abs() < 1e-9 {
            continue;
        }
        let far = if d[i] > 0.0 { hi[i] } else { lo[i] };
        exit = exit.min(((far - s[i]) / d[i]).max(0.0));
    }
    exit
}

/// If the ray enters a water volume before `trace` currently ends, rewrites
/// `trace` as a water-surface hit. Returns true if it did.
#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
pub(crate) unsafe fn apply_water_to_trace(
    vols: &[Volume],
    ray: *const u8,
    trace: *mut u8,
    world: usize,
) -> bool {
    unsafe { apply_water_to_trace_masked(vols, ray, trace, world, u32::MAX) }
}

/// Same, only for the volumes whose contents `mask` asks for.
pub(crate) unsafe fn apply_water_to_trace_masked(
    vols: &[Volume],
    ray: *const u8,
    trace: *mut u8,
    world: usize,
    mask: u32,
) -> bool {
    unsafe {
        apply_water_to_trace_at(
            vols,
            ray,
            trace,
            world,
            RAY_SWEPT_OFFSET.load(Ordering::Relaxed),
            mask,
        )
    }
}

/// The fields of a `Ray_t` the water logic needs.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RayFields {
    pub(crate) start: [f32; 3],
    pub(crate) delta: [f32; 3],
    pub(crate) offset: [f32; 3],
    pub(crate) extents: [f32; 3],
}

/// What a ray does to a trace result, decided by `plan_trace` and written by
/// `apply_water_to_trace_at`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum TraceOutcome {
    /// The ray starts and ends inside a volume: allsolid, fraction 0.
    Inside { origin: [f32; 3], contents: c_int },
    /// The ray starts inside a volume and leaves it: startsolid, plus the fraction at
    /// which it left (when that is earlier than what the trace already has).
    Leaves { left_at: Option<f32> },
    /// The ray enters a volume from outside. `fraction` and `end` are the entry point, or
    /// for a point ray (a bullet) `ENTRY_DEPTH` further in, so the engine's probe 0.1 below
    /// the end of the trace is inside the volume; `plane_dist` is the face's.
    Enters {
        origin: [f32; 3],
        end: [f32; 3],
        fraction: f32,
        axis: usize,
        min_face: bool,
        plane_dist: f32,
        contents: c_int,
    },
}

/// The water logic, on plain values: what `ray` does to a trace whose fraction is
/// `current`, counting only the volumes whose contents `mask` asks for. `None` leaves
/// the trace alone.
pub(crate) fn plan_trace(
    vols: &[Volume],
    ray: &RayFields,
    current: f32,
    mask: u32,
) -> Option<TraceOutcome> {
    let RayFields {
        start: s,
        delta: d,
        offset: off,
        extents: ext,
    } = *ray;
    let origin = [s[0] + off[0], s[1] + off[1], s[2] + off[2]];

    // Rays that START inside a volume. A real water brush reports these as
    // startsolid, and when the whole ray stays inside it as allsolid with
    // fraction 0 and the end point at the start. The engine's water-height
    // probe (a downward MASK_WATER ray from above the entity)
    // depends on that: without it a probe starting inside a volume looks
    // like "no water here" and the computed surface height collapses to
    // the entity's own height.
    let mut started_inside = false;
    let mut left_at = 1.0f32;
    for v in vols.iter().filter(|v| v.contents as u32 & mask != 0) {
        let lo = [v.min[0] - ext[0], v.min[1] - ext[1], v.min[2] - ext[2]];
        let hi = [v.max[0] + ext[0], v.max[1] + ext[1], v.max[2] + ext[2]];
        let inside = |p: [f32; 3]| (0..3).all(|i| p[i] >= lo[i] && p[i] <= hi[i]);
        if !inside(s) {
            continue;
        }
        started_inside = true;
        left_at = left_at.min(segment_exit_fraction(s, d, lo, hi));
        let e = [s[0] + d[0], s[1] + d[1], s[2] + d[2]];
        if inside(e) {
            return Some(TraceOutcome::Inside {
                origin,
                contents: v.contents,
            });
        }
    }
    if started_inside {
        // Where it left: the client's water explosion measures the depth from that.
        return Some(TraceOutcome::Leaves {
            left_at: (left_at < current).then_some(left_at),
        });
    }

    let mut best: Option<(f32, usize, bool, usize)> = None;
    for (i, v) in vols
        .iter()
        .enumerate()
        .filter(|(_, v)| v.contents as u32 & mask != 0)
    {
        let lo = [v.min[0] - ext[0], v.min[1] - ext[1], v.min[2] - ext[2]];
        let hi = [v.max[0] + ext[0], v.max[1] + ext[1], v.max[2] + ext[2]];
        if let Some((t, axis, min_face)) = segment_enters_box(s, d, lo, hi) {
            if t < current && best.is_none_or(|b| t < b.0) {
                best = Some((t, axis, min_face, i));
            }
        }
    }
    let (t_entry, axis, min_face, vi) = best?;
    let v = &vols[vi];
    // The engine decides whether a bullet hit water by testing the point 0.1 straight
    // below the end of this trace (`HandleShotImpactingWater`). A real water brush only
    // has a top face, so that point is always under the surface. A volume has side faces
    // too: a trace ending exactly on one has that point on the face's edge, or outside
    // it, and the shot gets no splash. End the trace slightly inside the volume instead.
    let lo = [v.min[0] - ext[0], v.min[1] - ext[1], v.min[2] - ext[2]];
    let hi = [v.max[0] + ext[0], v.max[1] + ext[1], v.max[2] + ext[2]];
    let t_exit = segment_exit_fraction(s, d, lo, hi);
    // Only for point rays (bullets). A hull (player movement, water-level probes) rests
    // against the face as before.
    let point_ray = ext.iter().all(|&e| e == 0.0);
    let push = if point_ray {
        ENTRY_DEPTH / d[axis].abs().max(1e-9)
    } else {
        0.0
    };
    let t = (t_entry + push)
        .min(f32::midpoint(t_entry, t_exit))
        .min((current - t_entry).mul_add(0.5, t_entry))
        .max(t_entry);
    let mut end = origin;
    for i in 0..3 {
        end[i] = d[i].mul_add(t, origin[i]);
    }
    Some(TraceOutcome::Enters {
        origin,
        end,
        fraction: t,
        axis,
        min_face,
        plane_dist: if min_face { -v.min[axis] } else { v.max[axis] },
        contents: v.contents,
    })
}

/// Same, with the offset of `Ray_t::m_IsSwept` given explicitly. Only the reads of the
/// ray and the writes of the trace are here; `plan_trace` decides what to write.
pub(crate) unsafe fn apply_water_to_trace_at(
    vols: &[Volume],
    ray: *const u8,
    trace: *mut u8,
    world: usize,
    swept_offset: usize,
    mask: u32,
) -> bool {
    unsafe {
        // A trace that reports a hit must name the entity it hit: callers
        // (the SLAM's beam, impact effects) call a virtual on `m_pEnt`. A real
        // water brush belongs to the world, so the world entity is used. If it
        // is unknown and the engine found no entity, leave the trace alone.
        let ent_slot = trace.add(0x58).cast::<usize>();
        let ent = if world != 0 {
            world
        } else {
            ent_slot.read_unaligned()
        };
        if ent == 0 {
            return false;
        }
        if ray.add(swept_offset).read() == 0 {
            return false; // not swept: nothing travels
        }
        let rf = |o: usize| ray.add(o).cast::<f32>().read_unaligned();
        let fields = RayFields {
            start: [rf(0x00), rf(0x04), rf(0x08)],
            delta: [rf(0x10), rf(0x14), rf(0x18)],
            offset: [rf(0x20), rf(0x24), rf(0x28)],
            extents: [rf(0x30), rf(0x34), rf(0x38)],
        };
        let current = trace.add(0x2c).cast::<f32>().read_unaligned();
        let Some(outcome) = plan_trace(vols, &fields, current, mask) else {
            return false;
        };

        let wf = |o: usize, val: f32| trace.add(o).cast::<f32>().write_unaligned(val);
        match outcome {
            TraceOutcome::Inside { origin, contents } => {
                for (i, &component) in origin.iter().enumerate() {
                    wf(i * 4, component);
                    wf(0x0c + i * 4, component);
                    wf(0x18 + i * 4, 0.0);
                }
                wf(0x24, 0.0);
                trace.add(0x28).write(0);
                trace.add(0x29).write(0);
                wf(0x2c, 0.0); // fraction
                trace.add(0x30).cast::<i32>().write_unaligned(contents);
                trace.add(0x34).cast::<u16>().write_unaligned(0);
                trace.add(0x36).write(1); // allsolid
                trace.add(0x37).write(1); // startsolid
                wf(0x38, 0.0);
                trace
                    .add(0x40)
                    .cast::<usize>()
                    .write_unaligned(WATER_SURFACE_NAME.as_ptr() as usize);
                ent_slot.write_unaligned(ent);
            }
            TraceOutcome::Leaves { left_at } => {
                trace.add(0x37).write(1);
                if let Some(fraction) = left_at {
                    wf(0x38, fraction);
                }
            }
            TraceOutcome::Enters {
                origin,
                end,
                fraction,
                axis,
                min_face,
                plane_dist,
                contents,
            } => {
                for i in 0..3 {
                    wf(i * 4, origin[i]); // startpos
                    wf(0x0c + i * 4, end[i]); // endpos
                    wf(0x18 + i * 4, 0.0); // plane.normal
                }
                wf(0x18 + axis * 4, if min_face { -1.0 } else { 1.0 });
                wf(0x24, plane_dist);
                trace.add(0x28).write(axis as u8); // plane.type: axial
                trace
                    .add(0x29)
                    .write(if min_face { 1u8 << axis } else { 0 }); // signbits
                wf(0x2c, fraction);
                trace.add(0x30).cast::<i32>().write_unaligned(contents);
                trace.add(0x34).cast::<u16>().write_unaligned(0); // dispFlags
                trace.add(0x36).write(0); // allsolid
                trace.add(0x37).write(0); // startsolid
                wf(0x38, 0.0); // fractionleftsolid
                trace
                    .add(0x40)
                    .cast::<usize>()
                    .write_unaligned(WATER_SURFACE_NAME.as_ptr() as usize);
                trace.add(0x48).cast::<i16>().write_unaligned(0); // surface.surfaceProps
                trace.add(0x4a).cast::<u16>().write_unaligned(0); // surface.flags
                trace.add(0x50).cast::<i32>().write_unaligned(0); // hitgroup
                trace.add(0x54).cast::<i16>().write_unaligned(0); // physicsbone
                trace.add(0x60).cast::<i32>().write_unaligned(0); // hitbox
                ent_slot.write_unaligned(ent); // m_pEnt
            }
        }
        true
    }
}

/// True if the ray certainly touches no volume: the box around its segment (grown by
/// the ray's extents) is separated from the box around every volume. Anything else,
/// NaN included, is "maybe". Reads the same ray fields as `apply_water_to_trace_at`.
pub(crate) unsafe fn ray_misses_bounds(
    bounds: Option<([f32; 3], [f32; 3])>,
    ray: *const u8,
) -> bool {
    unsafe {
        let rf = |o: usize| ray.add(o).cast::<f32>().read_unaligned();
        segment_misses_bounds(
            bounds,
            [rf(0x00), rf(0x04), rf(0x08)],
            [rf(0x10), rf(0x14), rf(0x18)],
            [rf(0x30), rf(0x34), rf(0x38)],
        )
    }
}

/// True if the segment from `start` along `delta`, grown by `extents`, certainly stays
/// outside `bounds` (no bounds: no volumes). NaN counts as "maybe".
pub(crate) fn segment_misses_bounds(
    bounds: Option<([f32; 3], [f32; 3])>,
    start: [f32; 3],
    delta: [f32; 3],
    extents: [f32; 3],
) -> bool {
    let Some((lo, hi)) = bounds else {
        return true; // no volumes
    };
    (0..3).any(|i| {
        let e = start[i] + delta[i];
        let (smin, smax) = if e < start[i] {
            (e, start[i])
        } else {
            (start[i], e)
        };
        smax < lo[i] - extents[i] || smin > hi[i] + extents[i]
    })
}

engine_abi_item! {
/// Replacement for `IEngineTrace::TraceRay`: runs the engine's trace, then, for a trace
/// with a water bit in its mask, lets `plan_trace` rewrite the result for the volumes the
/// ray touches. A trace without one is only remembered when it is a small hull (a shotgun
/// pellet), see `note_small_hull`.
pub(crate) unsafe fn hook_trace_ray(
    this: *mut c_void,
    ray: *const u8,
    mask: u32,
    filter: *mut c_void,
    trace: *mut u8,
) {
    unsafe {
        let original: engine_fn!((
            *mut c_void,
            *const u8,
            u32,
            *mut c_void,
            *mut u8,
        )) = std::mem::transmute(ORIGINAL_TRACE_RAY.load(Ordering::Relaxed));
        original(this, ray, mask, filter, trace);
        if ray.is_null() || trace.is_null() {
            return;
        }
        if mask & ((CONTENTS_WATER | CONTENTS_SLIME) as u32) == 0 {
            note_small_hull(ray, mask, trace);
            return;
        }
        // Never block inside an engine callback.
        // A ray far from every volume changes nothing (and needs no world entity).
        if let Some(vols) = snapshot_if(|v| !ray_misses_bounds(v.bounds, ray)) {
            let debug = DEBUG.load(Ordering::Relaxed);
            let before = if debug {
                Some(trace_summary(trace))
            } else {
                None
            };
            // Only the volumes whose contents the trace's mask asks for are considered.
            let world = world_entity();
            let hit = apply_water_to_trace_masked(&vols, ray, trace, world, mask);
            if hit {
                bump(&TRACE_HITS);
            }
            // A trace that changed nothing and started far from every volume (the sun's
            // line to the player, say) is not worth a log line.
            let start = [
                ray.cast::<f32>().read_unaligned(),
                ray.add(4).cast::<f32>().read_unaligned(),
                ray.add(8).cast::<f32>().read_unaligned(),
            ];
            let worth_logging = hit || vols.near(start, 2048.0);
            if let (Some((fraction, ent)), true) = (before, worth_logging) {
                let rf = |o: usize| ray.add(o).cast::<f32>().read_unaligned();
                let (after, _) = trace_summary(trace);
                debug_log(format!(
                    "trace mask {mask:#x} from ({:.3}, {:.3}, {:.3}) by ({:.3}, {:.3}, {:.3}) extents ({:.3}, {:.3}, {:.3}): swept {}, engine fraction {fraction:.5} entity {ent:#x}, world {world:#x} -> {} (fraction {after:.5}, startsolid {}, allsolid {})",
                    rf(0x00), rf(0x04), rf(0x08), rf(0x10), rf(0x14), rf(0x18),
                    rf(0x30), rf(0x34), rf(0x38),
                    ray.add(RAY_SWEPT_OFFSET.load(Ordering::Relaxed)).read(),
                    if hit { "rewritten" } else { "left alone" },
                    trace.add(0x37).read(),
                    trace.add(0x36).read(),
                ));
            }
        }
    }
}
}

thread_local! {
    /// The last small-hull trace that hit something near a volume: (its end point, the
    /// point where the bullet's own line reaches the surface the hull stopped at).
    static LAST_HULL: Cell<Option<([f32; 3], [f32; 3])>> = const { Cell::new(None) };
}

/// Where the centre line of a hull trace reaches the surface the hull stopped at. The
/// hull's centre rests `extents . |normal|` away from that surface, so for a bullet
/// that went into a volume through a side face and hit the floor next to it the centre
/// can end outside the volume while the bullet itself is inside.
pub(crate) fn hull_surface_point(
    end: [f32; 3],
    normal: [f32; 3],
    delta: [f32; 3],
    extents: [f32; 3],
) -> Option<[f32; 3]> {
    let len = delta[2]
        .mul_add(delta[2], delta[0].mul_add(delta[0], delta[1] * delta[1]))
        .sqrt();
    if len.is_nan() || len <= 1e-3 {
        return None;
    }
    let dir = [delta[0] / len, delta[1] / len, delta[2] / len];
    let toward = -dir[2].mul_add(normal[2], dir[0].mul_add(normal[0], dir[1] * normal[1]));
    if toward.is_nan() || toward <= 0.01 {
        return None;
    }
    let rest = extents[2].mul_add(
        normal[2].abs(),
        extents[1].mul_add(normal[1].abs(), extents[0] * normal[0].abs()),
    );
    let k = rest / toward;
    Some([
        dir[0].mul_add(k, end[0]),
        dir[1].mul_add(k, end[1]),
        dir[2].mul_add(k, end[2]),
    ])
}

/// A trace without the water bit whose hull is a few units wide (the shotgun's odd
/// pellets use a 3 unit hull) that hit something near a volume: remembers where its
/// line reaches the surface, for `hook_get_point_contents`, and logs it when debugging.
pub(crate) unsafe fn note_small_hull(ray: *const u8, mask: u32, trace: *const u8) {
    unsafe {
        let rf = |o: usize| ray.add(o).cast::<f32>().read_unaligned();
        let ext = [rf(0x30), rf(0x34), rf(0x38)];
        if !ext.iter().all(|&e| (1.0..=5.0).contains(&e)) {
            return;
        }
        let (fraction, ent) = trace_summary(trace);
        if fraction >= 1.0 {
            return;
        }
        if snapshot_if(|v| !ray_misses_bounds(v.bounds, ray)).is_none() {
            return;
        }
        let tf = |o: usize| trace.add(o).cast::<f32>().read_unaligned();
        let end = [tf(0x0c), tf(0x10), tf(0x14)];
        let normal = [tf(0x18), tf(0x1c), tf(0x20)];
        let delta = [rf(0x10), rf(0x14), rf(0x18)];
        if let Some(probe) = hull_surface_point(end, normal, delta, ext) {
            LAST_HULL.with(|c| c.set(Some((end, probe))));
        }
        if DEBUG.load(Ordering::Relaxed) {
            debug_log(format!(
                "hull trace mask {mask:#x} from ({:.3}, {:.3}, {:.3}) by ({:.3}, {:.3}, {:.3}) extents ({:.1}, {:.1}, {:.1}): fraction {fraction:.5} entity {ent:#x}, end ({:.3}, {:.3}, {:.3})",
                rf(0x00), rf(0x04), rf(0x08), delta[0], delta[1], delta[2],
                ext[0], ext[1], ext[2],
                end[0], end[1], end[2],
            ));
        }
    }
}

/// (fraction, hit entity) of a trace result.
pub(crate) const unsafe fn trace_summary(trace: *const u8) -> (f32, usize) {
    unsafe {
        (
            trace.add(0x2c).cast::<f32>().read_unaligned(),
            trace.add(0x58).cast::<usize>().read_unaligned(),
        )
    }
}

/// Swaps vtable slot `slot` of the `IEngineTrace` object for `hook`.
/// Returns (address of the slot, original function pointer).
pub(crate) unsafe fn install_slot_hook(
    trace: *mut c_void,
    slot: usize,
    hook: usize,
    original_store: &AtomicUsize,
    p: &Platform,
) -> Result<(usize, usize), &'static str> {
    unsafe {
        if trace.is_null() {
            return Err("enginetrace pointer is null");
        }
        let vtable = (trace as *const *mut usize).read();
        if vtable.is_null() {
            return Err("enginetrace has no vtable");
        }
        let entry = vtable.add(slot);
        let original = entry.read();
        if original == 0 || original == hook {
            return Err("vtable slot is empty or already hooked");
        }
        original_store.store(original, Ordering::SeqCst);
        let Some(old) = (p.make_writable)(entry.cast::<u8>(), 8) else {
            return Err("could not make the vtable writable");
        };
        entry.write(hook);
        (p.restore_protect)(entry.cast::<u8>(), 8, old);
        Ok((entry as usize, original))
    }
}

pub(crate) unsafe fn uninstall_slot_hook(entry: usize, hook: usize, original: usize, p: &Platform) {
    unsafe {
        let entry = entry as *mut usize;
        if entry.read() != hook {
            return; // somebody else replaced it after us; leave it alone
        }
        if let Some(old) = (p.make_writable)(entry.cast::<u8>(), 8) {
            entry.write(original);
            (p.restore_protect)(entry.cast::<u8>(), 8, old);
        }
    }
}

/// The world entity (entry 0 of the entity array), or 0 if unknown.
#[allow(clippy::needless_return)] // the server branch is the tail only when no client branch exists
pub(crate) unsafe fn world_entity() -> usize {
    unsafe {
        #[cfg(feature = "client")]
        {
            let list = CLIENT_ENTITY_LIST.load(Ordering::Relaxed);
            if list != 0 {
                // Client realm: entity 0 of the client's entity list.
                let vtable = (list as *const *const usize).read();
                let get: engine_fn!((usize, c_int) -> usize) =
                    std::mem::transmute(vtable.add(CLIENT_GET_ENTITY_SLOT).read());
                return get(list, 0);
            }
        }
        #[cfg(feature = "server")]
        {
            let g = ENTITY_ARRAY_GLOBAL.load(Ordering::Relaxed);
            if g == 0 {
                return 0;
            }
            let array = (g as *const usize).read_unaligned();
            if array == 0 {
                return 0;
            }
            return ((array + 8) as *const usize).read_unaligned();
        }
        #[cfg(not(feature = "server"))]
        0
    }
}
