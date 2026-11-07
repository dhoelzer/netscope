// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of netscope; see LICENSE.
//! Linking hints for libpcap.
//!
//! The `pcap` crate's own build script already finds libpcap through
//! `pkg-config` on Linux and against the system SDK on macOS, so this script
//! only covers the cases where that default detection is wrong:
//!
//! * Homebrew's `libpcap` is keg-only, so on Apple Silicon it lives in
//!   `/opt/homebrew` and is not on the default search path.
//! * Users who keep libpcap somewhere unusual can point us at it with
//!   `NETSCOPE_LIBPCAP_DIR`; we forward it as `LIBPCAP_LIBDIR`, which is the
//!   variable `pcap`'s build script reads to pick the right API level.
//!
//! On Windows nothing is linked at build time (`wpcap.dll` entry points are
//! resolved at runtime), so the script must stay out of the way there.

use std::env;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=NETSCOPE_LIBPCAP_DIR");
    println!("cargo:rerun-if-env-changed=LIBPCAP_LIBDIR");
    println!("cargo:rerun-if-env-changed=PKG_CONFIG_PATH");

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "windows" {
        return;
    }

    if let Some(dir) = requested_libpcap_dir() {
        println!("cargo:rustc-link-search=native={}", dir.display());
        println!("cargo:rustc-link-arg=-Wl,-rpath,{}", dir.display());
        // `cargo:rustc-env` only reaches this package's own compilation, so it
        // cannot hand LIBPCAP_LIBDIR to pcap's build script. Without it pcap
        // may version-probe a different libpcap than the one we link.
        if env::var_os("LIBPCAP_LIBDIR").is_none_or(|v| v.is_empty()) {
            emit(
                "warning",
                &format!(
                    "netscope: linking libpcap from {}, but LIBPCAP_LIBDIR is unset; \
                     re-run with `LIBPCAP_LIBDIR={} cargo build` if you see \
                     `cannot find function pcap_...` errors",
                    dir.display(),
                    dir.display()
                ),
            );
        }
        return;
    }

    if target_os == "macos" {
        for dir in ["/opt/homebrew/lib", "/usr/local/lib"] {
            if Path::new(dir).join("libpcap.dylib").exists() {
                println!("cargo:rustc-link-search=native={dir}");
                println!("cargo:rustc-link-arg=-Wl,-rpath,{dir}");
            }
        }
    }
}

/// Directory the user asked us to link libpcap from, if any.
fn requested_libpcap_dir() -> Option<PathBuf> {
    let raw = env::var_os("NETSCOPE_LIBPCAP_DIR")
        .or_else(|| env::var_os("LIBPCAP_LIBDIR"))
        .map(PathBuf::from)?;
    if raw.as_os_str().is_empty() {
        None
    } else {
        Some(raw)
    }
}

fn emit(directive: &str, message: &str) {
    println!("cargo:{directive}={message}");
}
