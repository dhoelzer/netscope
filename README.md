# netscope

A live network visualiser. It sniffs packets with **libpcap** (through the
[`pcap` crate](https://github.com/rust-pcap/pcap)) — either from a live
interface or by replaying a `.pcap` savefile — and draws who is talking to whom
with [**Vello CPU**](https://github.com/linebender/vello).

No GPU, no `wgpu`, no shaders: `vello_cpu` rasterises into a `Pixmap` that is
blitted into a `winit` window through `softbuffer`. That keeps the build story
on all three platforms down to one native library.

## What you see

```
 hops 20   16   12    8    4 1                  ┌─ this host ─────────┐
   |    |    |    |   |  | |                    │ ┌──┐ 192.168.1.20   │
 ( remote )      ( remote ) |                   │ └──┘     (global)   │
      |             |       |                   │ ┌──┐ 10.20.0.5      │
  ( remote )        |       |                   │ └──┘     (global)   │
      |             |       |                   │ ┌──┐ 169.254.3.3    │
      |             |       |                   │ └──┘   (link-local) │
      |             |       |                   └──────────┬──────────┘
  ( remote )        |       |                             |
                    |       |  (broadcast)        ╭────────┴───────╮
                    |       |     ◯               │   ◯      ◯     │
                    |       ╰─────────────────────│ local network  │
                    |                             │ 192.168.1.0/24 │
                    |                             ╰────────────────╯
```

Remote hosts are placed on the guide line matching their inferred hop count, so
a host one router away sits on line 1 and a host across the internet sits
further left. Broadcast and multicast groups are the exception: they sit in the
lane between the local-network circle and line 1, because they are local traffic
however their TTL reads.

* **Host panel (upper right)** — a vertical rectangle standing for the machine
  netscope runs on. Inside it, a stack of squares, one per address assigned to
  the host, each labelled with the address and its kind.
* **Local-network circle (lower right)** — a large circle for the local network,
  containing one smaller circle for every other host seen on it, labelled with
  its address. The caption carries the discovered subnet(s).
* **Remote circles (left side)** — one per unique host seen that is neither a
  local address nor on the local network. A circle stays on screen and fades out
  starting 30 s of real time after the last packet to or from that address.
* **Distance guide lines** — 20 vertical dashed lines, labelled `1`–`20` along
  the top, starting 25% of the window width in from the right edge and evenly
  spaced across the rest. They mark logical distance from the local network:
  a remote circle sits on the line matching the hop count implied by the TTL of
  the packets it sends us (TTL 64 ⇒ 1 hop, 63 ⇒ 2, 128 ⇒ 1, and so on). Only
  inbound packets count — the TTL *we* send says nothing about the path back.
  A host we have never heard from directly is parked at the far left.
* **Broadcast and multicast (near lane)** — the limited broadcast
  (`255.255.255.255`), the directed broadcast of one of our own subnets
  (`192.168.1.255` for a /24), IPv4 multicast (`224.0.0.0/4` and the rest of
  `224/4`), and IPv6 link- or site-scoped multicast are not single systems, so
  their TTL says nothing about distance. They get a circle of their own in the
  lane between the local-network circle and the 1-hop line — the near side of
  the ruler — instead of the far left edge. The local-network circle shrinks to
  keep that lane open.
* **Links** — one line per *connection*, from a remote circle to the *exact
  square* of the interface that carried the traffic, or out to the local-network
  circle when the remote reached another system on the LAN. Traffic between this
  host and the local network draws lines between the host panel and the
  local-network circle. LAN↔LAN traffic feeds the local-network circle; it never
  creates a remote circle.
* **Line colour** — set by protocol and port: `tcp/443`, `udp/53`, `icmp` and
  other well-known services have fixed colours, and anything else gets a stable
  colour derived from its protocol and port. A remote contacted over three
  services shows three coloured lines, fanned apart so all three stay visible.
  A colour key for the services currently on screen sits in the lower left.
* **Line fading** — unlike a host circle, a line starts fading the moment it is
  drawn and is gone after 45 s. Any further packet on that connection snaps it
  back to full brightness. Lines therefore disappear well before the circles
  they attach to.
* **Hover detail** — pointing at any shape brings up a card naming it: a remote
  circle shows its address, inferred distance (or that it is a group), packet
  and byte totals and how long since its last packet; an interface square shows
  the address, its kind and how many connections use it; a line shows the
  service and the traffic in each direction; the host panel and the
  local-network circle list what they stand for. The card is pushed clear of the
  shape and of the window edges, so it never hides the thing being inspected. A
  line's card follows the pointer along the line rather than sitting at one fixed
  end of it.

When there are more remote hosts than fit in one column, the grid wraps into
extra columns and tightens its spacing; anything that still cannot be drawn is
counted in the HUD (`· N not drawn`).

## Build

### Prerequisites

1. **Rust 1.89 or newer** (Vello CPU 0.3.0's MSRV; edition 2024). Install from
   <https://rustup.rs>:

   ```sh
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   rustc --version   # >= 1.89
   ```

2. **libpcap** (macOS/Linux) or **Npcap** (Windows) — see below.

### Linux

```sh
# Debian / Ubuntu
sudo apt update && sudo apt install -y build-essential pkg-config libpcap-dev

# Fedora / RHEL
sudo dnf install -y gcc pkgconf-pkg-config libpcap-devel

# Arch
sudo pacman -S --needed base-devel pkgconf libpcap
```

Then:

```sh
cargo build --release
```

`pkg-config` is what the `pcap` crate's build script uses to find and version-
probe libpcap, so install it alongside the headers.

### macOS

libpcap ships in the macOS SDK, so **nothing extra is required to build**:

```sh
xcode-select --install    # if you have no C toolchain yet
cargo build --release
```

If you installed libpcap from Homebrew (it is *keg-only* on Apple Silicon, so
it is not on the default search path), `build.rs` already adds `/opt/homebrew/lib`
to the linker search path and rpath when it finds `libpcap.dylib` there. To
point at a libpcap somewhere else:

```sh
brew install libpcap
NETSCOPE_LIBPCAP_DIR=/opt/homebrew/opt/libpcap/lib cargo build --release
```

### Windows

1. Install **Npcap** from <https://npcap.com/#download>. During setup, enable
   *"Install Npcap in WinPcap API-compatible Mode"*.
2. Install the Rust toolchain (`rustup-init.exe` from <https://rustup.rs>). The
   default `x86_64-pc-windows-msvc` host works; a C compiler is **not** needed
   for `pcap` itself, because no import library is linked at build time — the
   `wpcap.dll` entry points are resolved at runtime.

```powershell
cargo build --release
```

`build.rs` deliberately does nothing on Windows. If a call is missing from an
older `wpcap.dll`, `pcap` returns `Error::EntrypointNotFound` at runtime rather
than failing the compile.

### Permissions for live capture

Building is not the same as being allowed to sniff:

* **Linux** — either run with `sudo`, or grant the capability once:

  ```sh
  sudo setcap cap_net_raw,cap_net_admin=eip target/release/netscope
  ```

  Without it you get `libpcap error: socket: Operation not permitted`.
* **macOS** — capture devices are readable by the `_pcap` group (macOS 11+).
  Running from Terminal normally works; otherwise add yourself to `_pcap` or
  run with `sudo`.
* **Windows** — an elevated (Administrator) prompt is usually required unless
  Npcap was installed with the non-admin/"restricted" option.

## Run

```sh
# capture on the default interface
./target/release/netscope

# capture on a specific interface
./target/release/netscope --device eth0

# replay a savefile at 4× speed
./target/release/netscope --file capture.pcap --speed 4

# what can I capture on, and what addresses does it have?
./target/release/netscope --list-devices

# headless summary of a savefile, no window
./target/release/netscope --self-test capture.pcap
```

In the window: `q` / `Esc` quits, `Space` re-discovers the local interfaces
(useful after a VPN or Wi-Fi change).

### Testing the visual without live traffic

`--local` lets you describe addressing the machine is not actually attached to,
which makes the host/LAN/remote split reproducible anywhere:

```sh
cargo run --example dump -- out.png fixtures/sample.pcap \
    --local eth0/192.168.1.20/24 br0/10.20.0.5/16
```

`examples/capture-probe.rs` is a windowless live-capture smoke test — use it to
check that libpcap/Npcap and your permissions are fine before debugging the GUI:

```sh
cargo run --release --example capture-probe -- eth0 5
```

## Layout of the code

| Path | What lives there |
|---|---|
| `src/capture/parse.rs` | link-layer → IPv4/IPv6 → TCP/UDP decoding, incl. VLAN tags, IPv4 options, IPv6 extension headers |
| `src/capture/mod.rs` | capture thread: live device or savefile replay, stats, backpressure |
| `src/net.rs` | local interface/address discovery, subnet derivation, `Host` / `LocalNetwork` / `Remote` classification |
| `src/model.rs` | remote + LAN host tables, per-connection flow table, two fade clocks (30 s circles / 45 s lines), pruning, caps |
| `src/service.rs` | the service a line carries (protocol + port) and the colour it is drawn in |
| `src/render/layout.rs` | pure geometry: panel, squares, LAN circle, link endpoints |
| `src/render/text.rs` | skrifa font metrics + label fitting (glifo draws the glyphs) |
| `src/render/mod.rs` | Vello CPU scene: host panel, squares, LAN circle, links, HUD |
| `src/main.rs` | winit window, softbuffer present, argument parsing |
| `build.rs` | libpcap link/rpath hints (Homebrew keg-only, `NETSCOPE_LIBPCAP_DIR`) |
| `docs/reference/` | distilled API notes for pcap, Vello, Rust/Cargo |
| `fixtures/sample.pcap` | checked-in savefile so tests need no network |
| `scripts/make-fixture.py` | regenerates the fixture |

### Screenshot without a display

`examples/dump.rs` renders one frame straight to a PNG, which is how the layout
gets checked on a headless machine:

```sh
cargo run --release --example dump -- demo.png fixtures/sample.pcap \
    --local eth0/192.168.1.20/24 br0/10.20.0.5/16
```

The example carries its own minimal PNG writer (uncompressed deflate blocks) so
the crate has no image dependency; the output is therefore larger than a real
encoder would produce, and is git-ignored rather than committed.

## Tests

```sh
cargo test          # 98 tests: parsing, classification, TTL/hop inference, fade timing, colours, layout, hit-testing, render, end-to-end replay
cargo clippy --all-targets
```

The end-to-end test (`tests/pipeline.rs`) reads `fixtures/sample.pcap` through
the real `pcap` savefile reader, classifies every endpoint, and renders a frame
through Vello CPU. Tests never touch the network, so they pass unprivileged.

## Font

`assets/Roboto-Regular.ttf` (Apache-2.0, see `assets/LICENSE-Roboto.txt`) is
embedded with `include_bytes!` so the binary needs no font files at runtime.

## Licence

MIT OR Apache-2.0. The bundled Roboto font is Apache-2.0.
