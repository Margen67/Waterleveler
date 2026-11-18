// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67
// Embeds the build time so a stale module in the game folder is easy to spot.
fn main() {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    println!("cargo:rustc-env=WL_BUILD_TIME={secs}");
    println!("cargo:rerun-if-changed=src/lib.rs");
}
