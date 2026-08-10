// Copyright (C) 2026 SkapaCraft <https://skapacraft.com>
// SPDX-License-Identifier: GPL-3.0-or-later

use std::time::{SystemTime, UNIX_EPOCH};

/// Instant of the build, as a Unix timestamp, for `NOSTOS_BUILD_TIMESTAMP`.
///
/// The information panel tells the user how old the copy they are running is,
/// and this is the only way to know it without asking a server. The value stays
/// a bare integer: the date arithmetic happens in `app_state.rs` with the
/// `chrono` already in the dependency graph, so this script adds nothing to it.
///
/// `SOURCE_DATE_EPOCH` wins when it is set. Distribution builders export it to
/// make builds reproducible, and a timestamp read from the wall clock is
/// precisely the byte that would otherwise differ between two builds of the
/// same source.
fn build_timestamp() -> i64 {
    if let Some(epoch) = std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|value| value.trim().parse::<i64>().ok())
    {
        return epoch;
    }

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or_default()
}

fn main() {
    // `tauri_build` declares what the build depends on, so Cargo stops
    // re-running this script on every compile and the stamp can lag behind
    // during development. It costs nothing there and is always exact in the
    // packages that get distributed, which CI builds from a clean checkout.
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    println!(
        "cargo:rustc-env=NOSTOS_BUILD_TIMESTAMP={}",
        build_timestamp()
    );

    tauri_build::build()
}
