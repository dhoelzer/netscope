//! Local address / subnet classification tests.

use netscope::net::{
    AddressKind, LocalAddress, LocalNetwork, Location, format_address, format_subnet,
};
use std::net::IpAddr;

fn v4(ip: &str, prefix: u8, iface: &str) -> LocalAddress {
    LocalAddress {
        iface: iface.to_string(),
        ip: ip.parse().unwrap(),
        prefix: Some(prefix),
        kind: AddressKind::Global,
    }
}

#[test]
fn subnet_membership_and_host_vs_lan() {
    let net = LocalNetwork::from_addresses(vec![v4("192.168.1.20", 24, "wlan0")]);

    assert_eq!(net.subnets().len(), 1);
    assert_eq!(format_subnet(&net.subnets()[0]), "192.168.1.0/24");

    // Our own address is Host, not LocalNetwork.
    assert_eq!(
        net.classify(IpAddr::from([192, 168, 1, 20])),
        Location::Host
    );
    // Another box on the same /24 is a LAN peer.
    assert_eq!(
        net.classify(IpAddr::from([192, 168, 1, 7])),
        Location::LocalNetwork
    );
    // Anything outside is remote.
    assert_eq!(net.classify(IpAddr::from([8, 8, 8, 8])), Location::Remote);
    // The adjacent /24 is not ours.
    assert_eq!(
        net.classify(IpAddr::from([192, 168, 2, 5])),
        Location::Remote
    );
}

#[test]
fn multiple_interfaces_yield_multiple_subnets() {
    let net =
        LocalNetwork::from_addresses(vec![v4("10.0.0.5", 8, "eth0"), v4("172.16.4.9", 20, "br0")]);
    assert_eq!(net.subnets().len(), 2);
    assert_eq!(
        net.classify(IpAddr::from([10, 9, 9, 9])),
        Location::LocalNetwork
    );
    assert_eq!(
        net.classify(IpAddr::from([172, 16, 15, 254])),
        Location::LocalNetwork
    );
    assert_eq!(
        net.classify(IpAddr::from([172, 16, 32, 1])),
        Location::Remote
    );
}

#[test]
fn host_routes_do_not_swallow_single_hosts() {
    // A /32 (common on tunnels) must not create a subnet that matches only
    // itself; it should not appear as a subnet at all.
    let net = LocalNetwork::from_addresses(vec![v4("203.0.113.9", 32, "tun0")]);
    assert!(net.subnets().is_empty());
    assert_eq!(net.classify(IpAddr::from([203, 0, 113, 9])), Location::Host);
    assert_eq!(
        net.classify(IpAddr::from([203, 0, 113, 10])),
        Location::Remote
    );
}

#[test]
fn ipv6_subnet_and_link_local() {
    let net = LocalNetwork::from_addresses(vec![LocalAddress {
        iface: "eth0".into(),
        ip: "2001:db8::5".parse().unwrap(),
        prefix: Some(48),
        kind: AddressKind::Global,
    }]);
    assert_eq!(
        net.classify("2001:db8:0:ff::1".parse().unwrap()),
        Location::LocalNetwork
    );
    assert_eq!(
        net.classify("2001:db8:1::1".parse().unwrap()),
        Location::Remote
    );
    assert!(format_subnet(&net.subnets()[0]).ends_with("/48"));
}

#[test]
fn address_kinds_and_display_filtering() {
    let net = LocalNetwork::from_addresses(vec![
        LocalAddress {
            iface: "lo".into(),
            ip: "127.0.0.1".parse().unwrap(),
            prefix: Some(8),
            kind: AddressKind::Loopback,
        },
        LocalAddress {
            iface: "eth0".into(),
            ip: "169.254.5.5".parse().unwrap(),
            prefix: Some(16),
            kind: AddressKind::LinkLocal,
        },
        LocalAddress {
            iface: "eth0".into(),
            ip: "192.168.0.2".parse().unwrap(),
            prefix: Some(24),
            kind: AddressKind::Global,
        },
    ]);
    assert_eq!(net.loopback_addresses().len(), 1);
    // Loopback gets the inner circle, so it is excluded from the interface ring.
    assert_eq!(net.display_addresses().len(), 2);
    assert_eq!(
        format_address("192.168.0.2".parse().unwrap()),
        "192.168.0.2"
    );
    assert_eq!(
        format_address("::ffff:8.8.8.8".parse().unwrap()),
        "[v4-mapped 8.8.8.8]"
    );
}

#[test]
fn discovery_on_a_real_host_finds_something_or_is_empty() {
    // Whatever the machine looks like, discovery must not panic and must be
    // internally consistent.
    let net = LocalNetwork::discover().expect("discovery should not fail");
    for addr in net.addresses() {
        assert_eq!(net.classify(addr.ip), Location::Host);
    }
    for subnet in net.subnets() {
        assert!(subnet.prefix > 0);
        assert!(!net.subnets().is_empty());
    }
}
