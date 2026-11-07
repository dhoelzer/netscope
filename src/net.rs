// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of netscope; see LICENSE.
//! The host's own addresses and the local networks they imply.
//!
//! Two sources are merged: `if_addrs` (the OS interface table, which knows
//! about interfaces we are not capturing on) and `pcap::Device::addresses`
//! (which is authoritative for the devices libpcap can actually open). Merging
//! matters because on Windows `if_addrs` and Npcap disagree about which
//! adapters exist, and because a container often has addresses that libpcap
//! reports without a netmask.

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use crate::error::Error;

/// How an address assigned to this host should be treated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AddressKind {
    /// Loopback (127.0.0.0/8, ::1).
    Loopback,
    /// Link-local (169.254.0.0/16, fe80::/10).
    LinkLocal,
    /// Routable or private-but-routed address, e.g. a LAN or WAN address.
    Global,
}

/// An address assigned to an interface on this host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalAddress {
    pub iface: String,
    pub ip: IpAddr,
    /// CIDR prefix length; `None` when the source did not report a netmask.
    pub prefix: Option<u8>,
    pub kind: AddressKind,
}

/// A network block that counts as "the local network".
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Subnet {
    /// Network address, big-endian significant bits in the high position.
    pub network: u128,
    pub prefix: u8,
    pub is_v6: bool,
}

impl Subnet {
    /// The IPv4 directed broadcast address for this subnet, e.g. 192.168.1.255
    /// for 192.168.1.0/24. `None` for IPv6, which has no broadcast address.
    #[must_use]
    pub fn broadcast(&self) -> Option<Ipv4Addr> {
        if self.is_v6 || self.prefix == 0 {
            return None;
        }
        let host_bits = 32 - u32::from(self.prefix);
        let last = (self.network & 0xffff_ffff) | ((1u128 << host_bits) - 1);
        Some(Ipv4Addr::from(last as u32))
    }

    pub fn contains(&self, addr: IpAddr) -> bool {
        let (bits, is_v6) = to_bits(addr);
        if is_v6 != self.is_v6 || self.prefix >= width_bits(self.is_v6) {
            return false;
        }
        mask(bits, self.prefix, self.is_v6) == self.network
    }
}

/// Where an observed address sits relative to this host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Location {
    /// One of this host's own interface addresses.
    Host,
    /// Another system on one of this host's networks.
    LocalNetwork,
    /// A broadcast or multicast group: not a single system, and not one hop
    /// away in the way a unicast peer is, so it is grouped with the local
    /// network rather than placed on the hop ruler.
    LocalGroup,
    /// Everything else.
    Remote,
}

/// Snapshot of this host's addressing.
#[derive(Debug, Clone, Default)]
pub struct LocalNetwork {
    /// Interface addresses, ordered so the display is stable.
    addresses: Vec<LocalAddress>,
    subnets: Vec<Subnet>,
}

impl LocalNetwork {
    /// Collects addresses from the OS interface table and from libpcap.
    pub fn discover() -> Result<Self, Error> {
        let mut merged: BTreeMap<(String, IpAddr), LocalAddress> = BTreeMap::new();

        if let Ok(interfaces) = if_addrs::get_if_addrs() {
            for iface in interfaces {
                let (ip, prefix) = match &iface.addr {
                    if_addrs::IfAddr::V4(v4) => (IpAddr::V4(v4.ip), Some(v4.prefixlen)),
                    if_addrs::IfAddr::V6(v6) => (IpAddr::V6(v6.ip), Some(v6.prefixlen)),
                };
                merged.insert(
                    (iface.name.clone(), ip),
                    LocalAddress {
                        iface: iface.name,
                        ip,
                        prefix,
                        kind: classify_kind(ip),
                    },
                );
            }
        }

        // libpcap's device list is the second opinion; it also covers adapters
        // that `if_addrs` may hide on Windows.
        if let Ok(devices) = pcap::Device::list() {
            for device in devices {
                for address in device.addresses {
                    let key = (device.name.clone(), address.addr);
                    let entry = merged.entry(key).or_insert_with(|| LocalAddress {
                        iface: device.name.clone(),
                        ip: address.addr,
                        prefix: address.netmask.map(netmask_to_prefix).filter(|p| *p > 0),
                        kind: classify_kind(address.addr),
                    });
                    if entry.prefix.is_none() {
                        entry.prefix = address.netmask.map(netmask_to_prefix).filter(|p| *p > 0);
                    }
                }
            }
        }

        let addresses: Vec<LocalAddress> = merged.into_values().collect();
        let subnets = derive_subnets(&addresses);
        Ok(Self { addresses, subnets })
    }

    /// Parses `iface/ip/prefix` triples, used by `--local` to describe a
    /// network the host is not actually attached to.
    pub fn parse_overrides(specs: &[String]) -> Result<Self, Error> {
        let mut addresses = Vec::new();
        for spec in specs {
            let bad = || {
                Error::NoCaptureDevice(format!(
                    "bad --local {spec:?}; expected INTERFACE/ADDRESS[/PREFIX], e.g. eth0/192.168.1.20/24"
                ))
            };
            // Split from the right so an IPv6 address cannot eat the separator.
            let (iface, rest) = spec.split_once('/').ok_or_else(bad)?;
            if iface.is_empty() {
                return Err(bad());
            }
            let (ip, prefix) = match rest.rsplit_once('/') {
                Some((ip, prefix)) => (
                    ip.parse::<IpAddr>().map_err(|_| bad())?,
                    Some(prefix.parse::<u8>().map_err(|_| bad())?),
                ),
                None => (rest.parse::<IpAddr>().map_err(|_| bad())?, None),
            };
            if prefix.is_some_and(|p| p > width_bits(ip.is_ipv6())) {
                return Err(bad());
            }
            addresses.push(LocalAddress {
                iface: iface.to_string(),
                ip,
                prefix,
                kind: classify_kind(ip),
            });
        }
        if addresses.is_empty() {
            return Err(Error::NoCaptureDevice(
                "--local needs at least one INTERFACE/ADDRESS/PREFIX".into(),
            ));
        }
        Ok(Self::from_addresses(addresses))
    }

    /// Builds a network from an explicit list, used by tests and `--self-test`.
    #[must_use]
    pub fn from_addresses(addresses: Vec<LocalAddress>) -> Self {
        let subnets = derive_subnets(&addresses);
        Self { addresses, subnets }
    }

    #[must_use]
    pub fn addresses(&self) -> &[LocalAddress] {
        &self.addresses
    }

    #[must_use]
    pub fn subnets(&self) -> &[Subnet] {
        &self.subnets
    }

    #[must_use]
    pub fn is_host_address(&self, addr: IpAddr) -> bool {
        self.addresses.iter().any(|a| a.ip == addr)
    }

    #[must_use]
    pub fn is_local_network(&self, addr: IpAddr) -> bool {
        // A host address is never "another system on the local network".
        !self.is_host_address(addr) && self.subnets.iter().any(|s| s.contains(addr))
    }

    #[must_use]
    pub fn classify(&self, addr: IpAddr) -> Location {
        // Group before subnet: a subnet's own directed broadcast (192.168.1.255
        // for a /24) falls inside that subnet, but it names no single system, so
        // treating it as an ordinary peer would be wrong.
        if self.is_host_address(addr) {
            Location::Host
        } else if self.is_local_group(addr) {
            Location::LocalGroup
        } else if self.is_local_network(addr) {
            Location::LocalNetwork
        } else {
            Location::Remote
        }
    }

    /// Whether an address is a group address that only this host's own networks
    /// could have delivered: the limited broadcast, the directed broadcast of
    /// one of our subnets, IPv4 multicast, or IPv6 multicast scoped to our link
    /// or site.
    ///
    /// A group address is never a single system, so its TTL says nothing about
    /// how far away the sender is; that is why these are kept off the hop ruler.
    #[must_use]
    pub fn is_local_group(&self, addr: IpAddr) -> bool {
        match addr {
            IpAddr::V4(v4) => {
                v4.is_broadcast()
                    || v4.is_multicast()
                    || self
                        .subnets
                        .iter()
                        .any(|s| !s.is_v6 && s.broadcast() == Some(v4))
            }
            IpAddr::V6(v6) => {
                // A multicast address is ff | flags<<4 | scope in its first two
                // bytes. Only link-local (scope 2) and site-local (scope 5)
                // groups can reach us from our own networks; a realm-scoped
                // group is genuinely remote.
                let first = v6.segments()[0];
                first & 0xfff0 == 0xff00 && matches!(first & 0x000f, 0x0002 | 0x0005)
            }
        }
    }

    /// Interface addresses that should get their own circle in the middle ring.
    ///
    /// Link-local addresses are included (they are how LAN peers usually reach
    /// us) but loopback is excluded from the ring: it is shown as the inner
    /// circle instead.
    #[must_use]
    pub fn display_addresses(&self) -> Vec<&LocalAddress> {
        self.addresses
            .iter()
            .filter(|a| a.kind != AddressKind::Loopback)
            .collect()
    }

    #[must_use]
    pub fn loopback_addresses(&self) -> Vec<&LocalAddress> {
        self.addresses
            .iter()
            .filter(|a| a.kind == AddressKind::Loopback)
            .collect()
    }
}

fn classify_kind(ip: IpAddr) -> AddressKind {
    match ip {
        IpAddr::V4(v4) => {
            if v4.is_loopback() {
                AddressKind::Loopback
            } else if v4.is_link_local() {
                AddressKind::LinkLocal
            } else {
                AddressKind::Global
            }
        }
        IpAddr::V6(v6) => {
            if v6.is_loopback() {
                AddressKind::Loopback
            } else if v6.segments()[0] & 0xffc0 == 0xfe80 {
                AddressKind::LinkLocal
            } else {
                AddressKind::Global
            }
        }
    }
}

/// Turns each address with a known netmask into a subnet, dropping the
/// degenerate /32 and /128 host routes that would otherwise swallow single hosts.
fn derive_subnets(addresses: &[LocalAddress]) -> Vec<Subnet> {
    let mut subnets: Vec<Subnet> = addresses
        .iter()
        .filter_map(|a| {
            let prefix = a.prefix?;
            let (bits, is_v6) = to_bits(a.ip);
            if prefix == 0 || prefix >= width_bits(is_v6) {
                return None;
            }
            Some(Subnet {
                network: mask(bits, prefix, is_v6),
                prefix,
                is_v6,
            })
        })
        .collect();
    subnets.sort_unstable();
    subnets.dedup();
    subnets
}

fn to_bits(addr: IpAddr) -> (u128, bool) {
    match addr {
        IpAddr::V4(v4) => (u128::from(u32::from(v4)), false),
        IpAddr::V6(v6) => (u128::from(v6), true),
    }
}

/// Bit width an address of this family occupies inside the `u128` we store.
fn width_bits(is_v6: bool) -> u8 {
    if is_v6 { 128 } else { 32 }
}

/// Zeroes the low `width - prefix` bits, where `width` is 32 for IPv4 (stored
/// in the low half of the u128) and 128 for IPv6.
fn mask(bits: u128, prefix: u8, is_v6: bool) -> u128 {
    let width = u32::from(width_bits(is_v6));
    if prefix == 0 {
        return 0;
    }
    let shift = width - u32::from(prefix);
    let family_mask = if is_v6 { !0u128 } else { 0xffff_ffff };
    (bits & family_mask) & (!0u128 << shift) & family_mask
}

fn netmask_to_prefix(netmask: IpAddr) -> u8 {
    let (bits, is_v6) = to_bits(netmask);
    let total: u32 = if is_v6 { 128 } else { 32 };
    let trailing = bits.trailing_zeros().min(total);
    (total - trailing) as u8
}

/// Formats an address for a label, trimming an IPv6 link-local suffix noise.
#[must_use]
pub fn format_address(addr: IpAddr) -> String {
    match addr {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return format!("[v4-mapped {v4}]");
            }
            v6.to_string()
        }
    }
}

/// The subnet a display label should show for the middle ring.
#[must_use]
pub fn format_subnet(subnet: &Subnet) -> String {
    if subnet.is_v6 {
        let octets: [u8; 16] = u128::to_be_bytes(subnet.network);
        format!("{}/{}", Ipv6Addr::from(octets), subnet.prefix)
    } else {
        format!(
            "{}/{}",
            Ipv4Addr::from((subnet.network & 0xffff_ffff) as u32),
            subnet.prefix
        )
    }
}
