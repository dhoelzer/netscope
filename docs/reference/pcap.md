# pcap crate + libpcap — distilled reference

Sources: <https://github.com/rust-pcap/pcap> (README, `examples/*`, `src/*`),
<https://docs.rs/pcap>, libpcap man pages (`pcap(3PCAP)`, `pcap_open_live(3PCAP)`,
`pcap_open_offline(3PCAP)`, `pcap_next_ex(3PCAP)`).

Crate version used: **`pcap = "2.5.0"`** (Rust 2024, MSRV 1.85).
Native library: **libpcap** (macOS/Linux) / **Npcap** (Windows).

## Native library install (this is the whole build story)

| OS | Install | Notes |
|---|---|---|
| Debian/Ubuntu | `sudo apt install libpcap-dev pkg-config` | `pkg-config` is the recommended way for the crate's build script to find + version-probe libpcap. |
| Fedora/RHEL | `sudo dnf install libpcap-devel pkgconf-pkg-config` | |
| Arch | `sudo pacman -S libpcap pkgconf` | |
| macOS | `brew install libpcap` (optional) | libpcap ships in the macOS SDK, so **no install is required** for building. |
| Windows | Install **Npcap** from <https://npcap.com/#download> | Install with *"Install Npcap in WinPcap API-compatible Mode"*. No import lib is linked at build time — entry points are resolved at runtime from `wpcap.dll`, so **no Visual Studio C toolchain is needed** for `pcap` itself. |

### Permissions for live capture

* Linux, non-root: `sudo setcap cap_net_raw,cap_net_admin=eip target/binary`
  (README-documented). Or run with `sudo`.
* macOS: capture devices require the terminal/process to have been started from
  an account in the `_pcap` group (macOS 11+) or run as root; a freshly built
  binary run from a normal shell in Terminal usually works because Terminal is
  in `_pcap`.
* Windows: needs Npcap installed and typically an elevated (Administrator)
  prompt, unless Npcap was installed with the non-admin/"restricted" option.

### Version probing knobs (only needed if the build script guesses wrong)

* `LIBPCAP_LIBDIR=<dir>` — point at a libpcap in an unusual location. Ignored on
  Windows. If unset, the crate uses `pkg-config`.
* `LIBPCAP_VER=1.10.5` — force the API level (needed for static linking or when
  the version query fails, e.g. musl cross builds).
* Symptom of a wrong probe: `cannot find function 'pcap_<x>' in module 'raw'`.
* On Windows, calling an entry point missing from the installed `wpcap.dll`
  returns `Error::EntrypointNotFound` at runtime rather than failing the build.

## API surface actually used

```rust
use pcap::{Capture, Device, Error as PcapError, Linktype, Packet, PacketCodec,
           PacketHeader, Precision};
```

### Enumerating devices (and their addresses!)

```rust
let devices = pcap::Device::list()?;      // Vec<Device>
for d in &devices {
    // d.name: String, d.desc: Option<String>
    // d.flags: DeviceFlags  -> is_loopback(), is_up(), is_running(), is_wireless()
    // d.addresses: Vec<Address>
    for a in &d.addresses {
        // a.addr: IpAddr, a.netmask: Option<IpAddr>,
        // a.broadcast_addr: Option<IpAddr>, a.dst_addr: Option<IpAddr>
    }
}
```

`Device::lookup()` returns the *default* device (`Option<Device>`).

> `pcap::Device.addresses` gives address + netmask directly, which is enough to
> derive the local subnet. `if-addrs` is used in this project as a second source
> because it also exposes broadcast/point-to-point and interface-up state more
> reliably on Windows, and lets us enumerate addresses of interfaces we are not
> capturing on.

### Live capture

```rust
let mut cap = Capture::from_device(device)?      // Device: Into<Device>
    .promisc(true)
    .snaplen(96)                 // bytes of each frame to keep; we only need L3
    .timeout(200)                // ms; see gotcha below
    .immediate_mode(false)
    .open()?;                    // Capture<Active>
cap.filter("ip", true)?;         // BPF string, compiled by libpcap
```

Reading:

```rust
loop {
    match cap.next_packet() {           // -> Result<Packet<'_>, Error>
        Ok(p)  => { handle(p.header, p.data); }
        Err(PcapError::TimeoutExpired) => { /* no packet within timeout; keep looping */ }
        Err(e) => break,
    }
}
```

`Packet<'a> { header: &'a PacketHeader, data: &'a [u8] }` — **borrowed from the
capture handle**, so it must be consumed/copied before the next call.
`PacketHeader { ts: TimeVal, captured_len: i32, included_len: i32 }`.

Iterator form: `for pkt in cap.iter(Codec) { .. }` with a `PacketCodec` that
copies the bytes into an owned type (needed because the borrow cannot escape).

### Offline (pcap file) capture

```rust
let mut cap = Capture::from_file("capture.pcap")?;   // Capture<Offline>
// cap.version() -> (i32, i32)
loop {
    match cap.next_packet() {
        Ok(p) => handle(p.header, p.data),
        Err(PcapError::NoMorePackets) => break,      // EOF
        Err(e) => return Err(e.into()),
    }
}
```

`Capture::from_file_with_precision(path, Precision::Micro|Nano)` if timestamps
need normalising.

`Capture<Offline>` and `Capture<Active>` are **different types**; code that
handles both must be generic over `S: Activated` or boxed behind a trait object.
This project wraps both in an enum (`CaptureSource`) instead.

### Datalink / link layer

```rust
let lt: Linktype = cap.get_datalink();   // Linktype(pub i32)
```

Common constants: `Linktype::ETHERNET` (=1, EN10MB), `Linktype::RAW` (=101, raw
IPv4/IPv6, no L2 header), `Linktype::LOOP` (=108, BSD loopback, 4-byte address
family header), `Linktype::NULL` (=0, BSD loopback on macOS, 4-byte header),
`Linktype::IPV4`(=228)/`IPV6`(=229), `Linktype::LINUX_SLL`(=113, "cooked", 16-byte
header, common for `any`/`linux-virtual` capture devices),
`Linktype::IEEE802_11_RADIOTAP`(=127).

Because `Linktype` is a newtype over `i32`, match on the constants **and** fall
back to the numeric value. This project parses Ethernet (1), RAW (101),
NULL/LOOP (0/108), and LINUX_SLL (113); anything else is skipped.

### BPF filter strings worth using

`ip`, `ip or ip6`, `not ip6`, `ether host aa:bb:cc:dd:ee:ff`,
`net 192.168.0.0/24`, `port 443`. Compiling `ip or ip6` is the cheapest way to
drop ARP/ICMP-noise/LLDP from the stream before it reaches Rust.

## Gotchas

* **macOS + `timeout(0)`**: `next_packet()` can hang forever. Always use a
  non-zero timeout and loop (documented in the crate README).
* `TimeoutExpired` is a *normal* outcome for live capture with a timeout — never
  treat it as fatal.
* `Capture<T>` is `Send` but not `Sync`; run the capture on its own thread and
  ship parsed events over a channel.
* `snaplen` too small truncates payloads; 96 bytes covers Ethernet + IPv4 + TCP
  with options, which is all an IP-level visualiser needs.
* On Linux, `Device::list()` includes `any` and `linux-virtual` pseudo-devices
  that cannot be opened for normal capture — filter them out.
* Opening a device without permission yields `Error::NoDevs(...)` /
  "permission denied" — surface a hint about `setcap`/`sudo`/Npcap.
* `promisc(true)` needs `CAP_NET_RAW` too; it is not needed for our purposes.

## Corrections found while building this project (pcap 2.5.0)

* `Linktype` is a tuple struct: `pub struct Linktype(pub i32)`. Read the raw
  value with `cap.get_datalink().0`.
* `Capture::get_datalink` lives on `Capture<C: Activated>`; `Activated` is a
  trait implemented by `Active`, `Offline` and `Dead`, so a single helper can
  take `&Capture<pcap::Activated>` — but `get_datalink` itself is *not* a
  method on the `Activated` trait, so call it on the concrete capture.
* `pcap::Device::list()` gives `addresses: Vec<Address>` where `addr` is an
  `IpAddr` and `netmask`/`broadcast_addr`/`dst_addr` are `Option<IpAddr>` — no
  prefix length, so a subnet has to be derived from the netmask. `if-addrs`
  (`getifaddrs`) is the better source for local discovery: `Interface { name,
  addr: IfAddr, .. }` where `IfAddr::V4(Ifv4Addr { ip, netmask, prefixlen })`
  and `IfAddr::V6(Ifv6Addr { ip, netmask, prefixlen, scope })` give the prefix
  directly. pcap's device list is still needed to map names to capture devices.
* `Capture::from_file` + `next_packet()` is the simplest replay loop; the
  returned `Packet` borrows the capture, so copy `packet.data` before the next
  call.
