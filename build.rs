// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67
// Embeds the short commit hash as the build number, so a stale module in the game
// folder is easy to spot.
use std::process::Command;

fn main() {
    let from_git = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty());
    let from_env = std::env::var("GITHUB_SHA")
        .ok()
        .filter(|s| s.len() >= 7)
        .map(|s| s[..7].to_owned());
    let build = from_git
        .or(from_env)
        .unwrap_or_else(|| "unknown".to_owned());
    println!("cargo:rustc-env=WL_BUILD={build}");
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs/heads");
    println!("cargo:rerun-if-env-changed=GITHUB_SHA");
}
