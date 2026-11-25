// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67

use crate::*;

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

/// Offsets of every match of `pat` in `hay`.
pub(crate) fn find_all(hay: &[u8], pat: &[Option<u8>]) -> Vec<usize> {
    let mut out = Vec::new();
    if pat.is_empty() || hay.len() < pat.len() {
        return out;
    }
    let first = pat[0];
    'outer: for i in 0..=(hay.len() - pat.len()) {
        if let Some(b) = first {
            if hay[i] != b {
                continue;
            }
        }
        for (j, p) in pat.iter().enumerate().skip(1) {
            if let Some(b) = p {
                if hay[i + j] != *b {
                    continue 'outer;
                }
            }
        }
        out.push(i);
    }
    out
}

/// Every match of a signature in the executable parts of the module.
#[cfg(any(feature = "server", feature = "client"))]
pub(crate) unsafe fn scan_sig_all(
    base: *const u8,
    sig: &str,
) -> Result<Vec<*mut u8>, &'static str> {
    unsafe {
        let pat = parse_sig(sig);
        let mut found: Vec<*mut u8> = Vec::new();
        for (va, size) in executable_regions(base)? {
            let region = std::slice::from_raw_parts(base.add(va), size);
            for off in find_all(region, &pat) {
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
