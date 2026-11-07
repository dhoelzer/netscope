# netscope — agent notes

Live network visualiser: `pcap` capture → host/LAN/remote model → `vello_cpu`
rendering in a `winit` window via `softbuffer`.

## Build / test

```sh
export PATH="$HOME/.cargo/bin:$PATH"   # rustup toolchain, needs Rust >= 1.89
cargo test                             # 66 tests
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
  `Layout::hidden` (and `PlacedLan::hidden` for LAN peers) — never silently drop
  a host without incrementing the counter.
* Screen regions are fixed: host panel upper-right, LAN circle lower-right,
  remote columns on the left below `HUD_RESERVE`. `PlacedHost.rect` is the panel
  and `PlacedInterface.rect` is one square per *address* (`network.addresses()`,
  loopback included — not `display_addresses()`, which filters loopback out).
  `PlacedLanHost.offset` is **relative to the LAN centre**, not absolute.
* Link anchors go through `local_anchor` / `host_anchor` / `lan_anchor` so the
  line leaves the exact square that carried the traffic. `rect_edge` does the
  ray/rectangle intersection; `rim` does the circle case.

* **Two fade clocks.** Host circles hold for `FADE_SECONDS` (30 s) then fade over
  `FADE_TAIL_SECONDS`; traffic lines use `Model::link_opacity` and fade
  immediately over `LINK_FADE_SECONDS` (45 s). `prune()` applies a different
  horizon to each, so a line can vanish while its circle is still drawn.
* Flows and LAN links are keyed by **connection**, not host:
  `FlowKey = (remote, local, Service)`. `service_port()` names the service side
  (well-known port wins, else the smaller), which keeps both directions of one
  connection on one line. `MAX_FLOWS` bounds the table; call `enforce_flow_cap()`
  alongside `enforce_cap()`.
* `draw_links` groups connections by endpoint pair into a `BTreeMap` and fans
  parallel lines apart (`fan`, `LINK_FAN_GAP`). Sorting by `Service` is required,
  not cosmetic — HashMap order is random and an unordered fan flickers per frame.
* Interface square tints in `interface_tint` stay deliberately muted: a
  saturated fill there collides with protocol line colours and makes a line read
  as missing.

## Distance by TTL

* `Frame.ttl` carries the IPv4 TTL / IPv6 hop limit; `Model::infer_hops` turns it
  into a hop count. Only **inbound** packets from a remote reveal its distance —
  the TTL *we* transmit is our own default and says nothing about the return
  path, so outbound-only hosts have no inferred distance and park at the far left.
* Common OS defaults are 64 (Linux/macOS), 63, 128 (Windows) and 255 (network
  gear). Distance is `default - ttl`, so a TTL of 64 means 1 hop; `MAX_HOPS` (32)
  is the ceiling, and a TTL implying more yields `None` rather than a wrong number.
* `layout::hop_line_positions` owns the `HOP_LINES` (20) guide lines: the first
  sits 25% of the width in from the right, the rest are evenly spaced to the left
  margin. Render and tests both read it from there — never recompute the pitch in
  two places. `Layout::hop_lines` is the placed copy of the same positions.
* Broadcast and multicast (`Location::LocalGroup`) are **not** placed on the
  ruler: a group address is not one system, so its TTL says nothing about
  distance. They sit in the lane between the 1-hop line and the local-network
  circle (`Layout::near_x`, sized by `LAN_LANE_RESERVE`, which shrinks the circle
  to keep the lane open). `Model::group` marks them and suppresses hop inference.
* Dashes come from kurbo (`Stroke { dash_pattern, .. }`), not vello — see
  `docs/reference/vello.md`. Set `Cap::Butt` or round caps lengthen every dash.
* Label sizing is measured, not guessed: `Scene::remote_label_size` scales the
  font down to fit `Layout::label_budget` so a full IPv4 address stays legible
  instead of losing its last octet. `tests/render.rs` checks hop labels stay
  separate at several window widths, which is what caught the axis caption
  colliding with the hop numbers.

## Hover tooltips

* `Scene::draw` takes a `hover: Option<Point>` in **logical** window
  coordinates (winit gives physical ones; convert with `to_logical`). The tooltip
  is drawn last so it sits above the shape it names.
* `layout::hit_test` resolves the pointer to a `Hit`. Order matters and is
  deliberate: remotes, then LAN peer dots, then interface squares, then the host
  panel, then the LAN circle, then lines. Whatever is painted on top wins, so a
  peer dot inside the LAN circle reads as the peer.
* Lines are matched by distance to the segment, not by bounding box — a thin
  diagonal line's bbox is mostly empty space. `LINK_HIT_TOLERANCE` is the band;
  it widens by the line's own stroke width.
* Hit-testing uses `Layout::links`, filled **during** drawing by `stroke_fan`.
  That is why `draw_links` takes `&mut Layout`: a tooltip can only describe a
  line that was actually drawn this frame.
* `examples/dump.rs --hover TARGET` renders a card into a PNG for visual checks.
  Compare against a no-hover render of the same fixture to confirm a card appears.

## Scope

* Do not remove features from the working tree because they look unrelated to the
  task at hand. Hover tooltips were stripped as "scope creep" and had to be
  rebuilt; ask before deleting work you did not write.
