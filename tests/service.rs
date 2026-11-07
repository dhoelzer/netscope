// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of netscope; see LICENSE.
//! Line colouring: protocol/port keys, distinctness, and stability.

use netscope::capture::parse::Transport;
use netscope::service::{Service, WELL_KNOWN};

fn tcp(port: u16) -> Service {
    Service::new(Transport::Tcp, Some(port))
}

fn udp(port: u16) -> Service {
    Service::new(Transport::Udp, Some(port))
}

fn distinct(a: Service, b: Service) {
    let (x, y) = (a.rgb(), b.rgb());
    let spread = (x[0] - y[0]).abs() + (x[1] - y[1]).abs() + (x[2] - y[2]).abs();
    assert!(
        spread > 0.15,
        "{} and {} look too similar: {x:?} vs {y:?}",
        a.label(),
        b.label()
    );
}

#[test]
fn well_known_services_get_their_named_colours() {
    for (transport, port, name, colour) in WELL_KNOWN {
        let service = Service::new(*transport, *port);
        assert_eq!(service.rgb(), *colour, "colour for {name}");
        assert_eq!(service.described(), *name, "label for {name}");
    }
}

#[test]
fn icmp_and_dns_and_https_are_all_different_colours() {
    let icmp = Service::new(Transport::Icmp, None);
    let dns = udp(53);
    let https = tcp(443);
    distinct(icmp, dns);
    distinct(dns, https);
    distinct(icmp, https);
}

#[test]
fn tcp_and_udp_on_the_same_port_are_distinguishable() {
    distinct(tcp(443), udp(443));
    distinct(tcp(53), udp(53));
}

#[test]
fn colour_is_stable_for_a_given_service() {
    for service in [tcp(8080), udp(41234), Service::new(Transport::IcmpV6, None)] {
        assert_eq!(service.rgb(), service.rgb(), "{} unstable", service.label());
    }
}

#[test]
fn unlisted_ports_get_distinct_colours() {
    // Consecutive ephemeral ports must not collapse onto the same hue.
    let mut seen: Vec<([f32; 3], Service)> = Vec::new();
    for port in [1024u16, 1025, 1026, 31337, 45_000, 45_001] {
        let service = tcp(port);
        let rgb = service.rgb();
        for (other, name) in &seen {
            let spread =
                (rgb[0] - other[0]).abs() + (rgb[1] - other[1]).abs() + (rgb[2] - other[2]).abs();
            assert!(spread > 0.05, "tcp/{port} collides with {}", name.label());
        }
        seen.push((rgb, service));
    }
}

#[test]
fn derived_colours_stay_in_range_and_visible() {
    for port in 0..=65_535u16 {
        let [r, g, b] = tcp(port).rgb();
        for channel in [r, g, b] {
            assert!(
                (0.0..=1.0).contains(&channel),
                "channel out of range at {port}"
            );
        }
        // A black line on a dark background would be invisible.
        assert!(
            r.max(g).max(b) > 0.2,
            "tcp/{port} is too dark to see: {r},{g},{b}"
        );
    }
}

#[test]
fn labels_name_the_protocol_and_port() {
    assert_eq!(tcp(443).label(), "tcp/443");
    assert_eq!(udp(53).label(), "udp/53");
    assert_eq!(Service::new(Transport::Icmp, None).label(), "icmp");
    // Unlisted services fall back to the numeric label.
    assert_eq!(tcp(9999).described(), "tcp/9999");
    assert_eq!(tcp(443).described(), "tcp/443 https");
}

#[test]
fn portless_protocols_do_not_share_a_colour() {
    distinct(
        Service::new(Transport::Icmp, None),
        Service::new(Transport::IcmpV6, None),
    );
    distinct(
        Service::new(Transport::Icmp, None),
        Service::new(Transport::None, None),
    );
}
