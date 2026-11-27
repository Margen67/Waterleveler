// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67

use crate::*;

/// The alternatives of a signature: one per game build, separated by `|`.
pub(crate) fn alternatives(sig: &str) -> impl Iterator<Item = &str> {
    sig.split('|').map(str::trim)
}

pub(crate) fn parse_sig(sig: &str) -> Vec<Option<u8>> {
    sig.split_whitespace()
        .map(|b| {
            if b == "??" {
                None
            } else {
                Some(u8::from_str_radix(b, 16).expect("bad signature byte"))
            }
        })
        .collect()
}

/// The longest run of fixed (non-wildcard) bytes of `pat` as (start, length).
fn longest_fixed_run(pat: &[Option<u8>]) -> (usize, usize) {
    let (mut best, mut start) = ((0, 0), 0);
    for (i, b) in pat.iter().enumerate() {
        if b.is_none() {
            start = i + 1;
        } else if i + 1 - start > best.1 {
            best = (start, i + 1 - start);
        }
    }
    best
}

/// Offsets of every match of `pat` in `hay`, in order. `memchr` finds the longest fixed run
/// of the pattern (SIMD), and only the places where it occurs are checked against the
/// whole pattern, wildcards included.
pub(crate) fn find_all(hay: &[u8], pat: &[Option<u8>]) -> Vec<usize> {
    if pat.is_empty() || hay.len() < pat.len() {
        return Vec::new();
    }
    let last = hay.len() - pat.len();
    let (start, len) = longest_fixed_run(pat);
    if len == 0 {
        // nothing but wildcards: every position matches
        return (0..=last).collect();
    }
    let anchor: Vec<u8> = pat[start..start + len].iter().flatten().copied().collect();
    let finder = memchr::memmem::Finder::new(&anchor);
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(rel) = finder.find(&hay[from..]) {
        let at = from + rel;
        from = at + 1; // matches may overlap
        let Some(i) = at.checked_sub(start) else {
            continue;
        };
        if i > last {
            break;
        }
        if pat
            .iter()
            .zip(&hay[i..])
            .all(|(p, h)| p.is_none_or(|b| b == *h))
        {
            out.push(i);
        }
    }
    out
}

/// Every match of any alternative of `sig` in `hay`.
#[cfg(any(feature = "server", feature = "client", test))]
pub(crate) fn find_sig(hay: &[u8], sig: &str) -> Vec<usize> {
    let mut found: Vec<usize> = alternatives(sig)
        .flat_map(|alt| find_all(hay, &parse_sig(alt)))
        .collect();
    found.sort_unstable();
    found.dedup();
    found
}

/// Every match of a signature in the executable parts of the module.
#[cfg(any(feature = "server", feature = "client"))]
pub(crate) unsafe fn scan_sig_all(
    base: *const u8,
    sig: &str,
) -> Result<Vec<*mut u8>, &'static str> {
    unsafe {
        let mut found: Vec<*mut u8> = Vec::new();
        for (va, size) in executable_regions(base)? {
            let region = std::slice::from_raw_parts(base.add(va), size);
            for off in find_sig(region, sig) {
                found.push(base.add(va + off).cast_mut());
            }
        }
        if found.is_empty() {
            return Err("signature not found (different game build?)");
        }
        Ok(found)
    }
}

/// Same, for a signature that must match exactly once.
#[cfg(any(feature = "server", feature = "client"))]
pub(crate) unsafe fn scan_sig(base: *const u8, sig: &str) -> Result<*mut u8, &'static str> {
    unsafe {
        let found = scan_sig_all(base, sig)?;
        if found.len() == 1 {
            Ok(found[0])
        } else {
            Err("signature is ambiguous")
        }
    }
}
