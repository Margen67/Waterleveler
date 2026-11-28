// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67

use std::ffi::c_int;
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// Water volumes (platform neutral store)
// ---------------------------------------------------------------------------
//
// Axis-aligned boxes in world space. With the engine hook installed, every
// point inside a box reports Contents::WATER to the engine's own water logic.

bitflags::bitflags! {
    /// The engine's content flags (`CONTENTS_*`). Values read from the engine are converted
    /// with `from_bits_retain`, so bits this module does not name survive a round trip.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub(crate) struct Contents: c_int {
        const SOLID = 0x1;
        const SLIME = 0x10;
        const WATER = 0x20;
    }
}

pub(crate) const MAX_VOLUMES: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Volume {
    pub(crate) id: u32,
    pub(crate) min: [f32; 3],
    pub(crate) max: [f32; 3],
    /// What the engine sees inside: `Contents::WATER` or `Contents::SLIME`.
    pub(crate) contents: Contents,
}

/// The volumes plus one box around all of them, so a query far from every volume is
/// rejected with a single comparison instead of a scan. Only the methods below change the
/// list, and each one recomputes the box.
#[derive(Clone, Debug, Default)]
pub(crate) struct VolumeList {
    pub(crate) list: Vec<Volume>,
    /// The smallest box containing every volume, or `None` while the list is empty.
    pub(crate) bounds: Option<([f32; 3], [f32; 3])>,
    /// The same boxes as one array per field, so the point test runs over plain slices
    /// the compiler can vectorize.
    pub(crate) lanes: Lanes,
    /// A tree over the boxes, for lists too long to scan.
    pub(crate) tree: Bvh,
}

/// Lists up to this long are scanned (the array copy); longer ones go through the tree.
pub(crate) const TREE_MIN_VOLUMES: usize = 16;

/// Volumes per leaf of the tree.
pub(crate) const TREE_LEAF: usize = 4;

/// One node of the tree: a box around everything below it. A leaf (`count > 0`) holds
/// `order[a..a + count]`; any other node has children `a` and `b`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TreeNode {
    pub(crate) lo: [f32; 3],
    pub(crate) hi: [f32; 3],
    pub(crate) a: u32,
    pub(crate) b: u32,
    pub(crate) count: u32,
}

/// A bounding-volume hierarchy over the volume boxes (median split on the longest axis),
/// so a point far from most volumes only visits the few boxes near it.
#[derive(Clone, Debug, Default)]
pub(crate) struct Bvh {
    pub(crate) nodes: Vec<TreeNode>,
    /// Volume indices, grouped so every leaf is a contiguous run.
    pub(crate) order: Vec<u32>,
}

impl Bvh {
    pub(crate) const fn new() -> Self {
        Self {
            nodes: Vec::new(),
            order: Vec::new(),
        }
    }

    pub(crate) fn build(vols: &[Volume]) -> Self {
        let mut tree = Self::new();
        if vols.len() > TREE_MIN_VOLUMES {
            tree.order = (0..vols.len() as u32).collect();
            tree.split(vols, 0, vols.len());
        }
        tree
    }

    /// Builds the node for `order[from..to]` and returns its index.
    pub(crate) fn split(&mut self, vols: &[Volume], from: usize, to: usize) -> u32 {
        let mut lo = [f32::INFINITY; 3];
        let mut hi = [f32::NEG_INFINITY; 3];
        let mut centre_lo = [f32::INFINITY; 3];
        let mut centre_hi = [f32::NEG_INFINITY; 3];
        for &i in &self.order[from..to] {
            let v = &vols[i as usize];
            for axis in 0..3 {
                lo[axis] = lo[axis].min(v.min[axis]);
                hi[axis] = hi[axis].max(v.max[axis]);
                let c = v.min[axis] + v.max[axis];
                centre_lo[axis] = centre_lo[axis].min(c);
                centre_hi[axis] = centre_hi[axis].max(c);
            }
        }
        let index = self.nodes.len() as u32;
        if to - from <= TREE_LEAF {
            self.nodes.push(TreeNode {
                lo,
                hi,
                a: from as u32,
                b: 0,
                count: (to - from) as u32,
            });
            return index;
        }
        self.nodes.push(TreeNode {
            lo,
            hi,
            a: 0,
            b: 0,
            count: 0,
        });
        let axis = (0..3)
            .max_by(|&x, &y| {
                (centre_hi[x] - centre_lo[x]).total_cmp(&(centre_hi[y] - centre_lo[y]))
            })
            .unwrap_or(0);
        let mid = from + (to - from) / 2;
        // Only the halves matter, not the order inside them.
        self.order[from..to].select_nth_unstable_by(mid - from, |&x, &y| {
            let cx = vols[x as usize].min[axis] + vols[x as usize].max[axis];
            let cy = vols[y as usize].min[axis] + vols[y as usize].max[axis];
            cx.total_cmp(&cy)
        });
        let a = self.split(vols, from, mid);
        let b = self.split(vols, mid, to);
        let node = &mut self.nodes[index as usize];
        node.a = a;
        node.b = b;
        index
    }

    /// Calls `f` for every volume that contains `p`, until it returns false.
    pub(crate) fn visit(&self, vols: &[Volume], p: [f32; 3], mut f: impl FnMut(&Volume) -> bool) {
        // Median splits keep the tree shallow: the stack never holds more than depth + 1.
        let mut stack = [0u32; 64];
        let mut top = 1;
        if self.nodes.is_empty() {
            return;
        }
        while top > 0 {
            top -= 1;
            let node = &self.nodes[stack[top] as usize];
            if !(0..3).all(|i| p[i] >= node.lo[i] && p[i] <= node.hi[i]) {
                continue;
            }
            if node.count > 0 {
                for &i in &self.order[node.a as usize..(node.a + node.count) as usize] {
                    let v = &vols[i as usize];
                    if (0..3).all(|k| p[k] >= v.min[k] && p[k] <= v.max[k]) && !f(v) {
                        return;
                    }
                }
            } else {
                stack[top] = node.a;
                stack[top + 1] = node.b;
                top += 2;
            }
        }
    }
}

/// A struct-of-arrays copy of the volume list: entry `i` of every array is volume `i`.
#[derive(Clone, Debug, Default)]
pub(crate) struct Lanes {
    pub(crate) min: [Vec<f32>; 3],
    pub(crate) max: [Vec<f32>; 3],
    pub(crate) contents: Vec<c_int>,
}

impl VolumeList {
    pub(crate) const fn new() -> Self {
        Self {
            list: Vec::new(),
            bounds: None,
            lanes: Lanes {
                min: [Vec::new(), Vec::new(), Vec::new()],
                max: [Vec::new(), Vec::new(), Vec::new()],
                contents: Vec::new(),
            },
            tree: Bvh::new(),
        }
    }

    pub(crate) fn refresh_bounds(&mut self) {
        // Point queries use the lanes up to `TREE_MIN_VOLUMES` volumes and the tree above, so
        // only the one in use is built.
        if self.list.len() > TREE_MIN_VOLUMES {
            self.lanes = Lanes::default();
        } else {
            for axis in 0..3 {
                self.lanes.min[axis] = self.list.iter().map(|v| v.min[axis]).collect();
                self.lanes.max[axis] = self.list.iter().map(|v| v.max[axis]).collect();
            }
            self.lanes.contents = self.list.iter().map(|v| v.contents.bits()).collect();
        }
        self.tree = Bvh::build(&self.list);
        self.bounds = self.list.iter().fold(None, |acc, v| {
            Some(acc.map_or((v.min, v.max), |(lo, hi)| {
                (
                    [
                        lo[0].min(v.min[0]),
                        lo[1].min(v.min[1]),
                        lo[2].min(v.min[2]),
                    ],
                    [
                        hi[0].max(v.max[0]),
                        hi[1].max(v.max[1]),
                        hi[2].max(v.max[2]),
                    ],
                )
            }))
        });
    }

    pub(crate) fn push(&mut self, volume: Volume) {
        self.list.push(volume);
        self.refresh_bounds();
    }

    pub(crate) fn clear(&mut self) {
        self.list.clear();
        self.refresh_bounds();
    }

    pub(crate) fn retain(&mut self, keep: impl FnMut(&Volume) -> bool) {
        self.list.retain(keep);
        self.refresh_bounds();
    }

    /// Replaces every volume.
    #[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
    pub(crate) fn set(&mut self, volumes: Vec<Volume>) {
        self.list = volumes;
        self.refresh_bounds();
    }

    /// The contents of every volume containing `p`, or'ed together (0 if none). Same result
    /// as `contents_at` on the plain list, computed without a branch per volume.
    #[allow(clippy::needless_bitwise_bool)] // `&` on purpose: no early exit, so the loop vectorizes
    pub(crate) fn contents_at(&self, p: [f32; 3]) -> Contents {
        if self.list.len() > TREE_MIN_VOLUMES {
            let mut contents = Contents::empty();
            self.tree.visit(&self.list, p, |v| {
                contents |= v.contents;
                true
            });
            return contents;
        }
        let l = &self.lanes;
        let n = l.contents.len();
        let (x0, y0, z0) = (&l.min[0][..n], &l.min[1][..n], &l.min[2][..n]);
        let (x1, y1, z1) = (&l.max[0][..n], &l.max[1][..n], &l.max[2][..n]);
        let mut contents = 0;
        for i in 0..n {
            // `&`, not `&&`: no early exit, so the loop is a straight line of compares.
            let inside = (p[0] >= x0[i])
                & (p[0] <= x1[i])
                & (p[1] >= y0[i])
                & (p[1] <= y1[i])
                & (p[2] >= z0[i])
                & (p[2] <= z1[i]);
            contents |= l.contents[i] & -c_int::from(inside);
        }
        Contents::from_bits_retain(contents)
    }

    /// True if `p` is inside any volume.
    #[allow(clippy::needless_bitwise_bool)] // `&` on purpose: no early exit, so the loop vectorizes
    pub(crate) fn contains(&self, p: [f32; 3]) -> bool {
        if self.list.len() > TREE_MIN_VOLUMES {
            let mut inside = false;
            self.tree.visit(&self.list, p, |_| {
                inside = true;
                false // one is enough
            });
            return inside;
        }
        let l = &self.lanes;
        let n = l.contents.len();
        let (x0, y0, z0) = (&l.min[0][..n], &l.min[1][..n], &l.min[2][..n]);
        let (x1, y1, z1) = (&l.max[0][..n], &l.max[1][..n], &l.max[2][..n]);
        let mut inside = false;
        for i in 0..n {
            inside |= (p[0] >= x0[i])
                & (p[0] <= x1[i])
                & (p[1] >= y0[i])
                & (p[1] <= y1[i])
                & (p[2] >= z0[i])
                & (p[2] <= z1[i]);
        }
        inside
    }

    /// Top (surface height) of the highest water volume containing `p`.
    #[cfg(feature = "server")]
    pub(crate) fn surface_z(&self, p: [f32; 3]) -> Option<f32> {
        let mut top: Option<f32> = None;
        let mut take = |v: &Volume| {
            if v.contents.contains(Contents::WATER) {
                top = Some(top.map_or(v.max[2], |t| t.max(v.max[2])));
            }
            true
        };
        if self.list.len() > TREE_MIN_VOLUMES {
            self.tree.visit(&self.list, p, take);
        } else {
            for v in &self.list {
                if (0..3).all(|i| p[i] >= v.min[i] && p[i] <= v.max[i]) {
                    take(v);
                }
            }
        }
        top
    }

    /// False if `p` is certainly in no volume (true means "maybe": scan the list).
    pub(crate) fn may_contain(&self, p: [f32; 3]) -> bool {
        self.bounds
            .is_some_and(|(lo, hi)| (0..3).all(|i| p[i] >= lo[i] && p[i] <= hi[i]))
    }

    /// True if `p` is inside the box around every volume grown by `margin` (the debug log
    /// uses it to catch queries that land just outside a volume).
    pub(crate) fn near(&self, p: [f32; 3], margin: f32) -> bool {
        self.bounds.is_some_and(|(lo, hi)| {
            (0..3).all(|i| p[i] >= lo[i] - margin && p[i] <= hi[i] + margin)
        })
    }
}

impl std::ops::Deref for VolumeList {
    type Target = [Volume];

    fn deref(&self) -> &[Volume] {
        &self.list
    }
}

/// The writers' copy of the volumes. Edits go through `volumes()`, which publishes the
/// result to `SNAPSHOT` when the guard is released.
pub(crate) static VOLUMES: Mutex<VolumeList> = Mutex::new(VolumeList::new());

/// What the engine hooks read: an immutable copy of the list, replaced as a whole, so a
/// hook never waits for (or skips because of) an edit in progress. `None` while empty.
/// Reads and the replacement are lock-free.
pub(crate) static SNAPSHOT: arc_swap::ArcSwapOption<VolumeList> =
    arc_swap::ArcSwapOption::const_empty();

pub(crate) static NEXT_VOLUME_ID: std::sync::atomic::AtomicU32 =
    std::sync::atomic::AtomicU32::new(0);

pub(crate) static VOLUME_HITS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The volumes, locked for reading or editing. If the list was changed, the new copy is
/// published for the hooks when this is released.
pub(crate) struct VolumesGuard {
    pub(crate) guard: std::sync::MutexGuard<'static, VolumeList>,
    pub(crate) changed: bool,
}

impl std::ops::Deref for VolumesGuard {
    type Target = VolumeList;

    fn deref(&self) -> &VolumeList {
        &self.guard
    }
}

impl std::ops::DerefMut for VolumesGuard {
    fn deref_mut(&mut self) -> &mut VolumeList {
        self.changed = true;
        &mut self.guard
    }
}

impl Drop for VolumesGuard {
    fn drop(&mut self) {
        if self.changed {
            // Runs before the mutex is released, so copies are published in edit order.
            let copy = (!self.guard.is_empty()).then(|| Arc::new((*self.guard).clone()));
            SNAPSHOT.store(copy);
        }
    }
}

pub(crate) fn volumes() -> VolumesGuard {
    VolumesGuard {
        guard: VOLUMES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
        changed: false,
    }
}

/// The current volumes for an engine hook, or `None` when there are none. Never waits for an
/// edit.
#[cfg(all(test, target_os = "linux"))]
pub(crate) fn snapshot() -> Option<Arc<VolumeList>> {
    SNAPSHOT.load_full()
}

/// Like `snapshot`, but the copy is only taken (a pointer clone) when `maybe` says the query
/// could touch a volume, which is rare: `maybe` runs on the shared copy, so the common case
/// costs one lock-free load and a comparison.
pub(crate) fn snapshot_if(maybe: impl FnOnce(&VolumeList) -> bool) -> Option<Arc<VolumeList>> {
    SNAPSHOT
        .load()
        .as_ref()
        .filter(|v| maybe(v))
        .map(Arc::clone)
}

/// Runs `f` on the current volumes when `maybe` says the query could touch one, without taking
/// a copy of the pointer (no reference count changes): for a quick answer that is not kept.
/// Not for long work: the borrow holds up the next edit's cleanup of the old list.
pub(crate) fn snapshot_with<R>(
    maybe: impl FnOnce(&VolumeList) -> bool,
    f: impl FnOnce(&VolumeList) -> R,
) -> Option<R> {
    SNAPSHOT.load().as_ref().filter(|v| maybe(v)).map(|v| f(v))
}

/// Counts one event. A plain load and store, not an atomic add: these diagnostic counters
/// sit in hot hooks, and a count lost to a race between threads does not matter.
pub(crate) fn bump(counter: &std::sync::atomic::AtomicU64) {
    use std::sync::atomic::Ordering::Relaxed;
    counter.store(counter.load(Relaxed).wrapping_add(1), Relaxed);
}

/// Top (surface height) of the highest water volume containing `p` (slime is
/// not water: the engine's water explosion needs `Contents::WATER`).
#[cfg(all(test, target_os = "linux"))]
pub(crate) fn surface_z_at(vols: &[Volume], p: [f32; 3]) -> Option<f32> {
    vols.iter()
        .filter(|v| {
            v.contents.contains(Contents::WATER)
                && (0..3).all(|i| p[i] >= v.min[i] && p[i] <= v.max[i])
        })
        .map(|v| v.max[2])
        .fold(None, |a, z| Some(a.map_or(z, |a: f32| a.max(z))))
}

/// The contents of every volume containing `p`, or'ed together (0 if none). The reference
/// the tests hold `VolumeList::contents_at` to.
#[cfg(all(test, target_os = "linux"))]
pub(crate) fn contents_at(vols: &[Volume], p: [f32; 3]) -> Contents {
    vols.iter()
        .filter(|v| (0..3).all(|i| p[i] >= v.min[i] && p[i] <= v.max[i]))
        .fold(Contents::empty(), |c, v| c | v.contents)
}

#[cfg(all(test, target_os = "linux"))]
pub(crate) fn point_in_volumes(vols: &[Volume], p: [f32; 3]) -> bool {
    vols.iter()
        .any(|v| (0..3).all(|i| p[i] >= v.min[i] && p[i] <= v.max[i]))
}
