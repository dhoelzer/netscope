# Rust / Cargo — distilled reference

Sources: <https://rust-lang.org/learn/get-started/>,
<https://doc.rust-lang.org/cargo/index.html> (and *The Cargo Book* →
`reference/build-scripts`, `reference/profiles`, `reference/manifest`),
<https://doc.rust-lang.org/book/>.

## Toolchain

Install with rustup (not the distro package — distro Rust is usually stale and
Vello CPU needs a recent compiler):

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
# Windows: winget install Rustlang.Rustup  (or the rustup-init.exe installer)
# macOS:   brew install rustup-init && rustup-init
```

Vello CPU 0.3.0 needs **Rust ≥ 1.89**; `pcap` 2.5.0 needs **≥ 1.85** and uses
edition 2024. Verified working here: `rustc 1.99.1 / cargo 1.99.1`.

## Project layout used

```
Cargo.toml
build.rs              # optional, native-lib linking hints
src/main.rs           # binary crate root
src/<module>.rs       # modules declared with `mod x;` in main.rs
assets/               # include_bytes! targets
tests/                # integration tests (auto-discovered)
```

`cargo new --bin .` then edit `Cargo.toml`. `src/main.rs` = binary crate root;
`mod foo;` pulls in `src/foo.rs`.

## Cargo.toml essentials

```toml
[package]
name = "netscope"
version = "0.1.0"
edition = "2024"
rust-version = "1.89"

[dependencies]
pcap = "2.5.0"
vello_cpu = { version = "0.3.0", features = ["multithreading"] }
winit = "0.30.13"
softbuffer = "0.4.8"
if-addrs = "0.15.0"

[profile.release]
opt-level = 3
lto = "thin"
codegen-units = 1
```

* Version req `"2.5.0"` means `^2.5.0` (semver-compatible).
* `edition = "2024"` changes: `gen` is a keyword, `unsafe_op_in_unsafe_fn` is
  deny-by-default, tail-expression/temporary-lifetime rules tighten.
* Cargo feature unification is per-build, additive across the dependency graph.

## Useful commands

```sh
cargo check                 # type-check only, fastest
cargo clippy --all-targets  # lints
cargo fmt                   # format
cargo build --release
cargo run --release -- --file capture.pcap
cargo test                  # unit + integration
cargo doc --open
cargo tree                  # dependency graph (debug feature/linking issues)
cargo update -p crate --precise 1.2.3   # pin to satisfy an MSRV/compile error
```

## Build scripts (`build.rs`) — how native linking is controlled

`build.rs` runs before the crate compiles and communicates with Cargo by
printing `cargo:` directives to stdout:

```rust
println!("cargo:rustc-link-search=native=/opt/homebrew/lib");
println!("cargo:rustc-link-lib=dylib=pcap");
println!("cargo:rustc-link-arg=-Wl,-rpath,/opt/homebrew/lib");
println!("cargo:rerun-if-changed=build.rs");
println!("cargo:rerun-if-env-changed=LIBPCAP_LIBDIR");
println!("cargo:warning=netscope: linked libpcap {ver}");
```

Environment available to `build.rs`: `CARGO`, `CARGO_MANIFEST_DIR`,
`CARGO_CFG_TARGET_OS`, `CARGO_CFG_TARGET_ENV`, `OUT_DIR`,
`CARGO_PKG_VERSION`. Anything printed to **stdout** must be a `cargo:` directive;
use `cargo:warning=` or stderr for human messages.

Key facts for this project (from the `pcap` README):

* `pcap`'s own build script already probes libpcap via `pkg-config` and picks the
  right API level. **Most setups need no `build.rs` at all.**
* `LIBPCAP_LIBDIR` fixes version detection when libpcap lives somewhere unusual
  (Homebrew's `libpcap` is keg-only, so this matters on Apple Silicon if it is
  installed via brew).
* `LIBPCAP_VER` forces the API level for static/odd builds.
* On Windows nothing is linked at build time, so `build.rs` must **not** emit
  `rustc-link-lib` there.

So our `build.rs` only: (a) adds the Homebrew search path on macOS when present,
(b) sets `LIBPCAP_LIBDIR` if the user exported `NETSCOPE_LIBPCAP_DIR`,
(c) warns when libpcap cannot be found, and (d) is a no-op on Windows.

## Language notes relied on in this codebase

* Ownership/borrowing: `pcap::Packet<'_>` borrows the capture handle → copy the
  bytes out before the next `next_packet()`.
* `Send`/`Sync`: `Capture<T>` is `Send`, not `Sync` → dedicated capture thread.
* `std::sync::mpsc` (or a `crossbeam` channel) for capture→UI events.
* `std::time::Instant` for wall-clock fade timing; `Duration::as_secs_f64()`.
* `std::net::IpAddr`/`Ipv4Addr` for addresses; `Ipv4Addr::octets()`.
* Bit-twiddling subnet math on `u32` from `Ipv4Addr::from(u32)` /
  `u32::from(addr)` (big-endian order preserved by `Ipv4Addr`).
* `Option`/`Result` + `?` throughout; a single `thiserror`-free hand-written
  `Error` enum keeps deps minimal.
* `#[cfg(target_os = "...")]` for platform-specific code and `build.rs` branches.
