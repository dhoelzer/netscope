//! Model behaviour: classification, flow anchoring, fade timing, pruning.

use netscope::capture::Sample;
use netscope::capture::parse::{Frame, Transport};
use netscope::model::{FADE_SECONDS, FADE_TAIL_SECONDS, LINK_FADE_SECONDS, Model};
use netscope::net::{AddressKind, LocalAddress, LocalNetwork};
use std::net::IpAddr;
use std::time::{Duration, Instant};

fn net() -> LocalNetwork {
    LocalNetwork::from_addresses(vec![
        LocalAddress {
            iface: "eth0".into(),
            ip: "192.168.1.20".parse().unwrap(),
            prefix: Some(24),
            kind: AddressKind::Global,
        },
        LocalAddress {
            iface: "lo".into(),
            ip: "127.0.0.1".parse().unwrap(),
            prefix: Some(8),
            kind: AddressKind::Loopback,
        },
    ])
}

fn ip(s: &str) -> IpAddr {
    s.parse().unwrap()
}

fn frame(src: &str, dst: &str, proto: u8) -> Frame {
    Frame {
        src: ip(src),
        dst: ip(dst),
        src_port: Some(1),
        dst_port: Some(2),
        transport: if proto == 6 {
            Transport::Tcp
        } else {
            Transport::Udp
        },
        wire_len: 60,
    }
}

#[test]
fn remote_host_gets_a_circle_and_a_flow_to_the_interface() {
    let network = net();
    let mut model = Model::default();
    let t0 = Instant::now();

    model.ingest(
        &Sample {
            frame: frame("192.168.1.20", "93.184.216.34", 6),
        },
        &network,
        t0,
    );

    assert_eq!(model.remote_count(), 1);
    let remote = model.remotes().next().unwrap();
    assert_eq!(remote.addr, ip("93.184.216.34"));
    assert_eq!(remote.packets, 1);
    assert_eq!(remote.lan_packets, 0);

    let flows: Vec<_> = model.flows().collect();
    assert_eq!(flows.len(), 1);
    // The local side is our own interface, so the link goes to the middle ring.
    assert_eq!(flows[0].local, Some(ip("192.168.1.20")));
}

#[test]
fn remote_talking_to_a_lan_peer_anchors_on_the_lan_circle() {
    let network = net();
    let mut model = Model::default();
    let t0 = Instant::now();

    model.ingest(
        &Sample {
            frame: frame("10.5.5.5", "192.168.1.7", 17),
        },
        &network,
        t0,
    );

    assert_eq!(model.remote_count(), 1);
    assert_eq!(model.lan_count(), 1);
    let remote = model.remotes().next().unwrap();
    assert_eq!(remote.lan_packets, 1);

    let flows: Vec<_> = model.flows().collect();
    assert_eq!(flows.len(), 1);
    // `None` means "draw the line to the local-network circle".
    assert_eq!(flows[0].local, None);
}

#[test]
fn lan_to_lan_traffic_creates_no_remote_circle() {
    let network = net();
    let mut model = Model::default();
    let t0 = Instant::now();

    model.ingest(
        &Sample {
            frame: frame("192.168.1.7", "192.168.1.9", 6),
        },
        &network,
        t0,
    );

    assert_eq!(model.remote_count(), 0);
    assert_eq!(model.lan_count(), 2);
}

#[test]
fn remote_to_remote_creates_two_circles() {
    let network = net();
    let mut model = Model::default();
    let t0 = Instant::now();

    model.ingest(
        &Sample {
            frame: frame("8.8.8.8", "1.1.1.1", 17),
        },
        &network,
        t0,
    );

    assert_eq!(model.remote_count(), 2);
}

#[test]
fn repeated_packets_refresh_the_clock_and_accumulate() {
    let network = net();
    let mut model = Model::default();
    let t0 = Instant::now();
    let f = frame("192.168.1.20", "93.184.216.34", 6);

    for i in 0..5 {
        model.ingest(
            &Sample { frame: f },
            &network,
            t0 + Duration::from_secs(i * 10),
        );
    }
    let now = t0 + Duration::from_secs(45);
    let remote = model.remotes().next().unwrap();
    assert_eq!(remote.packets, 5);
    // Last packet at t0+40, so at t0+45 the age is 5s: still fully visible.
    assert_eq!(Model::opacity(remote.last_seen, now), 1.0);
}

#[test]
fn lan_peer_talking_to_our_interface_creates_a_lan_link() {
    let network = net();
    let mut model = Model::default();
    let t0 = Instant::now();

    model.ingest(
        &Sample {
            frame: frame("192.168.1.7", "192.168.1.20", 6),
        },
        &network,
        t0,
    );

    // No remote circle: both ends are local.
    assert_eq!(model.remote_count(), 0);
    assert_eq!(model.lan_count(), 1);

    // But there is a link between the interface ring and the LAN circle.
    let links: Vec<_> = model.lan_links().collect();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].peer, ip("192.168.1.7"));
    assert_eq!(links[0].local, ip("192.168.1.20"));
    assert_eq!(links[0].packets, 1);
}

#[test]
fn lan_to_lan_traffic_creates_no_lan_link() {
    let network = net();
    let mut model = Model::default();
    let t0 = Instant::now();
    model.ingest(
        &Sample {
            frame: frame("192.168.1.7", "192.168.1.9", 6),
        },
        &network,
        t0,
    );
    assert_eq!(model.lan_links().count(), 0);
}

#[test]
fn lan_links_are_pruned_on_the_line_clock_not_the_host_clock() {
    let network = net();
    let mut model = Model::default();
    let t0 = Instant::now();
    model.ingest(
        &Sample {
            frame: frame("192.168.1.7", "192.168.1.20", 6),
        },
        &network,
        t0,
    );
    assert_eq!(model.lan_links().count(), 1);

    // Past the host circle's window but still inside the line's 45s fade.
    model.prune(t0 + Duration::from_secs_f64(FADE_SECONDS + FADE_TAIL_SECONDS + 0.5));
    assert_eq!(model.lan_links().count(), 1, "line still fading");

    model.prune(t0 + Duration::from_secs_f64(LINK_FADE_SECONDS + 0.5));
    assert_eq!(model.lan_links().count(), 0);
}

#[test]
fn link_opacity_fades_immediately_and_reaches_zero_at_45s() {
    let t0 = Instant::now();
    assert_eq!(Model::link_opacity(t0, t0), 1.0);

    // No hold period: already dimmed after a quarter of the window.
    let quarter = Model::link_opacity(t0, t0 + Duration::from_secs_f64(LINK_FADE_SECONDS / 4.0));
    assert!(
        (quarter - 0.75).abs() < 1e-6,
        "expected ~0.75 a quarter in, got {quarter}"
    );

    let mid = Model::link_opacity(t0, t0 + Duration::from_secs_f64(LINK_FADE_SECONDS / 2.0));
    assert!((mid - 0.5).abs() < 1e-6, "expected ~0.5 halfway, got {mid}");

    assert_eq!(
        Model::link_opacity(t0, t0 + Duration::from_secs_f64(LINK_FADE_SECONDS)),
        0.0
    );
    assert_eq!(
        Model::link_opacity(t0, t0 + Duration::from_secs_f64(LINK_FADE_SECONDS * 2.0)),
        0.0
    );
}

#[test]
fn a_further_packet_refreshes_a_line_to_full_brightness() {
    let network = net();
    let mut model = Model::default();
    let t0 = Instant::now();

    model.ingest(
        &Sample {
            frame: frame("192.168.1.20", "93.184.216.34", 6),
        },
        &network,
        t0,
    );
    let at = t0 + Duration::from_secs(40);
    let faded = model
        .flows()
        .map(|f| Model::link_opacity(f.last_seen, at))
        .next()
        .unwrap();
    assert!(
        faded < 0.15,
        "after 40s the line should be nearly gone: {faded}"
    );

    // Another packet on the same connection resets it to full brightness.
    model.ingest(
        &Sample {
            frame: frame("192.168.1.20", "93.184.216.34", 6),
        },
        &network,
        at,
    );
    let refreshed = model
        .flows()
        .map(|f| Model::link_opacity(f.last_seen, at))
        .next()
        .unwrap();
    assert_eq!(refreshed, 1.0);
    assert_eq!(model.flows().count(), 1, "refresh reuses the same flow");
}

#[test]
fn flows_carry_the_protocol_and_service_port_that_colour_the_line() {
    let network = net();
    let mut model = Model::default();
    let t0 = Instant::now();

    // Outbound HTTPS from our interface to the remote.
    model.ingest(
        &Sample {
            frame: Frame {
                src: ip("192.168.1.20"),
                dst: ip("93.184.216.34"),
                src_port: Some(51000),
                dst_port: Some(443),
                transport: Transport::Tcp,
                wire_len: 60,
            },
        },
        &network,
        t0,
    );
    let flow = model.flows().next().unwrap();
    assert_eq!(flow.service.transport, Transport::Tcp);
    // The service port, not our ephemeral one, so both directions agree.
    assert_eq!(flow.service.port, Some(443));

    // The reply in the other direction keeps the same service port.
    model.ingest(
        &Sample {
            frame: Frame {
                src: ip("93.184.216.34"),
                dst: ip("192.168.1.20"),
                src_port: Some(443),
                dst_port: Some(51000),
                transport: Transport::Tcp,
                wire_len: 60,
            },
        },
        &network,
        t0,
    );
    let flows: Vec<_> = model.flows().collect();
    assert_eq!(flows.len(), 1, "both directions are one connection");
    assert_eq!(flows[0].service.port, Some(443));
}

#[test]
fn different_services_to_one_remote_are_separate_lines() {
    let network = net();
    let mut model = Model::default();
    let t0 = Instant::now();

    for (proto, dst_port) in [(6u8, 443u16), (17, 53), (6, 80)] {
        model.ingest(
            &Sample {
                frame: Frame {
                    src: ip("192.168.1.20"),
                    dst: ip("93.184.216.34"),
                    src_port: Some(50_000),
                    dst_port: Some(dst_port),
                    transport: if proto == 6 {
                        Transport::Tcp
                    } else {
                        Transport::Udp
                    },
                    wire_len: 60,
                },
            },
            &network,
            t0,
        );
    }

    // One circle, but one line per connection, each with its own colour.
    assert_eq!(model.remote_count(), 1);
    let flows: Vec<_> = model.flows().collect();
    assert_eq!(flows.len(), 3);
    let services: Vec<_> = flows.iter().map(|f| f.service).collect();
    assert_eq!(
        services
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        3
    );
}

#[test]
fn the_service_port_prefers_the_well_known_side() {
    let network = net();
    let mut model = Model::default();
    // A LAN peer reaching our ssh port: the line should read tcp/22, not the
    // peer's ephemeral port.
    model.ingest(
        &Sample {
            frame: Frame {
                src: ip("192.168.1.7"),
                dst: ip("192.168.1.20"),
                src_port: Some(4514),
                dst_port: Some(22),
                transport: Transport::Tcp,
                wire_len: 60,
            },
        },
        &network,
        Instant::now(),
    );
    let link = model.lan_links().next().unwrap();
    assert_eq!(link.service.port, Some(22));
    assert_eq!(link.service.label(), "tcp/22");
}

#[test]
fn flow_cap_evicts_the_stalest_connections() {
    let network = net();
    let mut model = Model::default();
    let t0 = Instant::now();
    for i in 0..(netscope::model::MAX_FLOWS + 25) {
        model.ingest(
            &Sample {
                frame: Frame {
                    src: ip("192.168.1.20"),
                    dst: ip("93.184.216.34"),
                    src_port: Some(40_000),
                    dst_port: Some(1_024 + (i % 60_000) as u16),
                    transport: Transport::Udp,
                    wire_len: 60,
                },
            },
            &network,
            t0 + Duration::from_secs(i as u64),
        );
    }
    model.enforce_flow_cap();
    assert_eq!(model.flows().count(), netscope::model::MAX_FLOWS);
}

#[test]
fn icmp_flows_have_no_port() {
    let network = net();
    let mut model = Model::default();
    model.ingest(
        &Sample {
            frame: Frame {
                src: ip("192.168.1.20"),
                dst: ip("93.184.216.34"),
                src_port: None,
                dst_port: None,
                transport: Transport::Icmp,
                wire_len: 60,
            },
        },
        &network,
        Instant::now(),
    );
    let flow = model.flows().next().unwrap();
    assert_eq!(flow.service.transport, Transport::Icmp);
    assert_eq!(flow.service.port, None);
}

#[test]
fn opacity_fades_after_the_window_and_reaches_zero() {
    let t0 = Instant::now();
    assert_eq!(Model::opacity(t0, t0), 1.0);
    assert_eq!(
        Model::opacity(t0, t0 + Duration::from_secs_f64(FADE_SECONDS)),
        1.0
    );
    let mid = Model::opacity(t0, t0 + Duration::from_secs_f64(FADE_SECONDS + 2.0));
    assert!(mid > 0.4 && mid < 0.6, "mid-fade should be ~0.5, got {mid}");
    assert_eq!(
        Model::opacity(
            t0,
            t0 + Duration::from_secs_f64(FADE_SECONDS + FADE_TAIL_SECONDS)
        ),
        0.0
    );
}

#[test]
fn stale_hosts_are_pruned_only_after_the_fade_completes() {
    let network = net();
    let mut model = Model::default();
    let t0 = Instant::now();
    model.ingest(
        &Sample {
            frame: frame("192.168.1.20", "93.184.216.34", 6),
        },
        &network,
        t0,
    );

    // Still inside the 30s window: nothing to prune.
    model.prune(t0 + Duration::from_secs(10));
    assert_eq!(model.remote_count(), 1);

    // Fading but not gone.
    model.prune(t0 + Duration::from_secs_f64(FADE_SECONDS + 1.0));
    assert_eq!(model.remote_count(), 1);

    // Past the fade tail: the circle is gone. The line outlives it, because
    // lines fade on their own 45s clock.
    model.prune(t0 + Duration::from_secs_f64(FADE_SECONDS + FADE_TAIL_SECONDS + 0.5));
    assert_eq!(model.remote_count(), 0);
    assert_eq!(model.flows().count(), 1, "line still fading");

    model.prune(t0 + Duration::from_secs_f64(LINK_FADE_SECONDS + 0.5));
    assert_eq!(model.flows().count(), 0);
}

#[test]
fn quiet_for_reports_idle_time() {
    let network = net();
    let mut model = Model::default();
    let t0 = Instant::now();
    assert!(!model.quiet_for(t0).is_finite());

    model.ingest(
        &Sample {
            frame: frame("192.168.1.20", "93.184.216.34", 6),
        },
        &network,
        t0,
    );
    let idle = model.quiet_for(t0 + Duration::from_secs(3));
    assert!((idle - 3.0).abs() < 0.01, "expected ~3s, got {idle}");
}

#[test]
fn host_cap_evicts_the_stalest_remotes() {
    let network = net();
    let mut model = Model::default();
    let t0 = Instant::now();
    for i in 0..(netscope::model::MAX_REMOTE_HOSTS + 25) {
        let addr = format!("203.0.113.{}", i % 251);
        let peer = format!("198.51.100.{}", (i / 251) % 251);
        model.ingest(
            &Sample {
                frame: frame(&addr, &peer, 6),
            },
            &network,
            t0 + Duration::from_millis(i as u64),
        );
    }
    model.enforce_cap();
    assert!(model.remote_count() <= netscope::model::MAX_REMOTE_HOSTS);
    // The freshest host must survive.
    assert!(model.remotes().any(|h| h.addr == ip("203.0.113.0")));
}
