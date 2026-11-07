#!/usr/bin/env python3
"""Regenerate fixtures/sample.pcap.

The fixture is checked in so `cargo test` needs no network access and no
libpcap-injected traffic. It deliberately mixes the cases the visualiser has
to get right:

  1. host interface -> remote            (link to the middle interface ring)
  2. remote -> host interface            (same link, other direction)
  3. remote -> LAN peer                  (link to the local-network circle)
  4. LAN peer -> LAN peer                (no remote circle at all)
  5. LAN peer -> host interface          (link between interface ring and LAN circle)
  6. ARP frame                           (not IP: must be skipped)
  7. host -> remote over IPv6            (v6 addressing)
  8. a truncated frame                   (must be skipped, not panic)

Run from the repository root:

    python3 scripts/make-fixture.py
"""

import struct
import sys
from pathlib import Path

EN10MB = 1
LINKTYPE = EN10MB  # Ethernet, matching what most capture interfaces report


def ethernet(src: str, dst: str, ethertype: int, body: bytes) -> bytes:
    return bytes.fromhex(src) + bytes.fromhex(dst) + struct.pack("!H", ethertype) + body


def ipv4(src: str, dst: str, proto: int, payload: bytes) -> bytes:
    src_b = bytes(int(p) for p in src.split("."))
    dst_b = bytes(int(p) for p in dst.split("."))
    total = 20 + len(payload)
    header = struct.pack(
        "!BBHHHBBH",
        0x45,  # version 4, IHL 5
        0,  # DSCP/ECN
        total,
        0x1234,  # identification
        0x4000,  # flags = DF, fragment offset 0
        64,  # TTL
        proto,
        0,  # checksum (the decoder does not validate it)
    )
    return header + src_b + dst_b + payload


def tcp(src_port: int, dst_port: int) -> bytes:
    return struct.pack("!HHIIBBHHH", src_port, dst_port, 0, 0, 0x50, 0x10, 65535, 0, 0)


def udp(src_port: int, dst_port: int) -> bytes:
    return struct.pack("!HHHH", src_port, dst_port, 9, 0) + b"x"


def ipv6(src: bytes, dst: bytes, next_header: int, payload: bytes) -> bytes:
    return (
        struct.pack("!I", 0x60000000)
        + struct.pack("!H", len(payload))
        + bytes([next_header, 64])
        + src
        + dst
        + payload
    )


def v6_global(tail: int) -> bytes:
    return bytes([0x20, 0x01, 0x0D, 0xB8]) + b"\x00" * 11 + bytes([tail])


def build() -> bytes:
    host = "192.168.1.20"
    lan_peer_a = "192.168.1.7"
    lan_peer_b = "192.168.1.9"
    remote_web = "93.184.216.34"
    remote_dns = "10.5.5.5"
    bridge = "10.20.0.5"

    host_mac, router_mac = "aabbccddeeff", "112233445566"

    packets = [
        ethernet(host_mac, router_mac, 0x0800, ipv4(host, remote_web, 6, tcp(51000, 443))),
        # Second interface talking to a remote, so the demo shows links leaving
        # different squares of the host panel.
        ethernet(host_mac, router_mac, 0x0800, ipv4(bridge, remote_dns, 17, udp(5353, 53))),
        ethernet(router_mac, host_mac, 0x0800, ipv4(remote_web, host, 6, tcp(443, 51000))),
        ethernet(router_mac, host_mac, 0x0800, ipv4(remote_dns, lan_peer_a, 17, udp(53, 5353))),
        ethernet(host_mac, router_mac, 0x0800, ipv4(lan_peer_a, lan_peer_b, 6, tcp(9000, 22))),
        ethernet(router_mac, host_mac, 0x0800, ipv4(lan_peer_a, host, 6, tcp(4514, 22))),
        ethernet(host_mac, "ffffffffffff", 0x0806, b"\x00" * 28),
        ethernet(
            host_mac, router_mac, 0x86DD, ipv6(v6_global(1), v6_global(2), 6, tcp(4000, 80))
        ),
        b"\x00\x01\x02",  # truncated: too short for any link header
    ]

    global_header = struct.pack("<IHHiIII", 0xA1B2C3D4, 2, 4, 0, 0, 262144, LINKTYPE)
    out = bytearray(global_header)
    base = 1_700_000_000
    for index, packet in enumerate(packets):
        out += struct.pack("<IIII", base + index, 0, len(packet), len(packet))
        out += packet
    return bytes(out)


def main() -> int:
    out = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("fixtures/sample.pcap")
    out.parent.mkdir(parents=True, exist_ok=True)
    data = build()
    out.write_bytes(data)
    print(f"wrote {out} ({len(data)} bytes)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
