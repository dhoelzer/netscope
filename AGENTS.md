# netscope — agent notes

Live network visualiser: `pcap` capture → host/LAN/remote model → `vello_cpu`
rendering in a `winit` window via `softbuffer`.

## Build / test

```sh
export PATH="$HOME/.cargo/bin:$PATH"   # rustup toolchain, needs Rust >= 1.89
cargo test                             # 39 tests
cargo clippy --all-targets             # must stay clean
cargo fmt                              # not --check; CI-less repo, keep formatted
cargo run --quiet -- --list-devices    # does not need capture permission
cargo run --quiet --example dump -- out.png fixtures/sample.pcap --local eth0/192.168.1.20/24
```

* Live capture needs `CAP_NET_RAW`; in this sandbox it fails with
  `libpcap error: socket: Operation not permitted`. That is expected — do not
  chase it. Test the capture path with `--file`/`--self-test` instead.
* Tests must never touch the network or need privileges. Use
  `fixtures/sample.pcap` (regenerate with `scripts/make-fixture.py`) and
  `LocalNetwork::parse_overrides` rather than real interface discovery.

## Reference material

`docs/reference/{pcap,vello,rust-cargo}.md` hold the distilled API notes, each
with a "Corrections found while building this project" section. **Read those
corrections before touching render or capture code** — several signatures differ
from what the public docs pages show (`Pixmap::data()` is `&[PremulRgba8]`,
`Linktype` is a tuple struct, skrifa advances are already in pixels, colours
come from `vello_cpu::color::{AlphaColor, Srgb}`).

## Architecture invariants

* Time is always the UI arrival clock (`Instant`), never pcap timestamps. The
  30 s fade (`model::FADE_SECONDS`) plus 4 s tail is the spec'd visibility
  window; `prune` uses that same horizon for remotes, LAN peers and LAN links.
* `net::LocalNetwork::classify` returns `Host` / `LocalNetwork` / `Remote`.
  That one function decides every placement decision; classification bugs show
  up as circles on the wrong side.
* LAN peer ↔ our interface is a separate `model::LanLink` (interface ring →
  LAN circle), distinct from a remote `Flow`. LAN↔LAN traffic feeds the LAN
  circle only and must never create a remote circle.
* `render/layout.rs` is pure geometry and unit-testable; `render/mod.rs` only
  draws. Layout wraps remotes into columns and reports overflow in
  `Layout::hidden` — never silently drop a host without incrementing it.
