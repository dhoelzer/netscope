//! Minimal L2/L3/L4 decoding of a captured frame.
//!
//! We only need the IP endpoints (plus ports and a protocol tag for the HUD),
//! so `snaplen` can stay small and anything we do not understand is skipped
//! rather than treated as an error.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// Data-link layer of a capture, resolved from `pcap::Linktype`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkLayer {
    Ethernet,
    /// Raw IP, no link-layer header (DLT_RAW).
    RawIp,
    /// BSD loopback (DLT_NULL / DLT_LOOP): native-endian address family.
    NullLoopback,
    /// Linux "cooked" v1 (DLT_LINUX_SLL).
    LinuxSll,
    Unsupported(i32),
}

impl LinkLayer {
    #[must_use]
    pub fn from_pcap_value(value: i32) -> Self {
        match value {
            1 => LinkLayer::Ethernet,
            101 => LinkLayer::RawIp,
            0 | 108 => LinkLayer::NullLoopback,
            113 => LinkLayer::LinuxSll,
            other => LinkLayer::Unsupported(other),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Transport {
    Tcp,
    Udp,
    Icmp,
    IcmpV6,
    Other(u8),
    None,
}

/// A decoded frame: the two IP endpoints and whatever transport detail is useful.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame {
    pub src: IpAddr,
    pub dst: IpAddr,
    pub src_port: Option<u16>,
    pub dst_port: Option<u16>,
    pub transport: Transport,
    /// IP-level length reported by the wire, not the captured length.
    pub wire_len: u32,
}

/// Decodes one captured frame. Returns `None` for anything that is not IP.
#[must_use]
pub fn decode(link: LinkLayer, bytes: &[u8]) -> Option<Frame> {
    let (ethertype, rest) = strip_link_layer(link, bytes)?;
    match ethertype {
        ETHERTYPE_IPV4 => decode_ipv4(rest),
        ETHERTYPE_IPV6 => decode_ipv6(rest),
        _ => None,
    }
}

const ETHERTYPE_IPV4: u16 = 0x0800;
const ETHERTYPE_IPV6: u16 = 0x86dd;
const ETHERTYPE_VLAN: u16 = 0x8100;
const ETHERTYPE_QINQ: u16 = 0x88a8;

fn strip_link_layer(link: LinkLayer, bytes: &[u8]) -> Option<(u16, &[u8])> {
    match link {
        LinkLayer::Ethernet => {
            if bytes.len() < 14 {
                return None;
            }
            let mut ethertype = u16::from_be_bytes([bytes[12], bytes[13]]);
            let mut rest = &bytes[14..];
            // Walk VLAN tags so tagged traffic still decodes.
            while matches!(ethertype, ETHERTYPE_VLAN | ETHERTYPE_QINQ) {
                if rest.len() < 4 {
                    return None;
                }
                ethertype = u16::from_be_bytes([rest[2], rest[3]]);
                rest = &rest[4..];
            }
            Some((ethertype, rest))
        }
        LinkLayer::RawIp => match *bytes.first()? >> 4 {
            4 => Some((ETHERTYPE_IPV4, bytes)),
            6 => Some((ETHERTYPE_IPV6, bytes)),
            _ => None,
        },
        LinkLayer::NullLoopback => {
            if bytes.len() < 4 {
                return None;
            }
            // DLT_NULL stores the family in host order; AF_INET is 2 on every
            // platform we target, AF_INET6 is 30 on BSD and 10 on Linux.
            let family = u32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
            let rest = &bytes[4..];
            match family {
                2 => Some((ETHERTYPE_IPV4, rest)),
                10 | 30 => Some((ETHERTYPE_IPV6, rest)),
                _ => None,
            }
        }
        LinkLayer::LinuxSll => {
            if bytes.len() < 16 {
                return None;
            }
            Some((u16::from_be_bytes([bytes[14], bytes[15]]), &bytes[16..]))
        }
        LinkLayer::Unsupported(_) => None,
    }
}

fn decode_ipv4(bytes: &[u8]) -> Option<Frame> {
    if bytes.len() < 20 || bytes[0] >> 4 != 4 {
        return None;
    }
    let ihl = usize::from(bytes[0] & 0x0f) * 4;
    if ihl < 20 || bytes.len() < ihl {
        return None;
    }
    let total_len = u16::from_be_bytes([bytes[2], bytes[3]]) as u32;
    let protocol = bytes[9];
    let frag_off = u16::from_be_bytes([bytes[6], bytes[7]]);
    let src = IpAddr::V4(Ipv4Addr::from([bytes[12], bytes[13], bytes[14], bytes[15]]));
    let dst = IpAddr::V4(Ipv4Addr::from([bytes[16], bytes[17], bytes[18], bytes[19]]));
    // Only the first fragment carries transport headers.
    let (transport, src_port, dst_port) = if frag_off & 0x1fff == 0 {
        decode_transport(protocol, &bytes[ihl..])
    } else {
        (transport_of(protocol), None, None)
    };
    Some(Frame {
        src,
        dst,
        src_port,
        dst_port,
        transport,
        wire_len: total_len,
    })
}

fn decode_ipv6(bytes: &[u8]) -> Option<Frame> {
    if bytes.len() < 40 || bytes[0] >> 4 != 6 {
        return None;
    }
    let payload_len = u16::from_be_bytes([bytes[4], bytes[5]]) as usize;
    let src_octets: [u8; 16] = bytes[8..24].try_into().ok()?;
    let dst_octets: [u8; 16] = bytes[24..40].try_into().ok()?;
    let mut next = bytes[6];
    let mut cursor = 40usize;
    // Hop through extension headers until we reach the upper protocol.
    for _ in 0..8 {
        // Length of the extension header that starts at `cursor`, in bytes.
        let ext_len = match next {
            0 | 43 | 60 => (usize::from(bytes.get(cursor + 1).copied().unwrap_or(0)) + 1) * 8,
            51 => 8, // fragment header is fixed length
            _ => break,
        };
        if cursor + ext_len > bytes.len() {
            return None;
        }
        next = bytes[cursor];
        cursor += ext_len;
    }
    let end = (cursor + payload_len).min(bytes.len());
    let (transport, src_port, dst_port) = decode_transport(next, &bytes[cursor..end]);
    Some(Frame {
        src: IpAddr::V6(Ipv6Addr::from(src_octets)),
        dst: IpAddr::V6(Ipv6Addr::from(dst_octets)),
        src_port,
        dst_port,
        transport,
        wire_len: 40 + payload_len as u32,
    })
}

fn transport_of(protocol: u8) -> Transport {
    match protocol {
        6 => Transport::Tcp,
        17 => Transport::Udp,
        1 => Transport::Icmp,
        58 => Transport::IcmpV6,
        other => Transport::Other(other),
    }
}

fn decode_transport(protocol: u8, payload: &[u8]) -> (Transport, Option<u16>, Option<u16>) {
    let transport = transport_of(protocol);
    if matches!(transport, Transport::Tcp | Transport::Udp) && payload.len() >= 4 {
        (
            transport,
            Some(u16::from_be_bytes([payload[0], payload[1]])),
            Some(u16::from_be_bytes([payload[2], payload[3]])),
        )
    } else {
        (transport, None, None)
    }
}
