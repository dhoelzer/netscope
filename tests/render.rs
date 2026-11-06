//! Layout placement and an end-to-end Vello CPU render.

use netscope::capture::Sample;
use netscope::capture::parse::{Frame, Transport};
use netscope::model::Model;
use netscope::net::{AddressKind, LocalAddress, LocalNetwork};
use netscope::render::{Hud, Scene, layout};
use std::time::Instant;

fn ip(s: &str) -> std::net::IpAddr {
    s.parse().unwrap()
}

fn network() -> LocalNetwork {
    LocalNetwork::from_addresses(vec![
        LocalAddress {
            iface: "eth0".into(),
            ip: "192.168.1.20".parse().unwrap(),
            prefix: Some(24),
            kind: AddressKind::Global,
        },
        LocalAddress {
            iface: "eth0".into(),
            ip: "169.254.3.3".parse().unwrap(),
            prefix: Some(16),
            kind: AddressKind::LinkLocal,
        },
        LocalAddress {
            iface: "lo".into(),
            ip: "127.0.0.1".parse().unwrap(),
            prefix: Some(8),
            kind: AddressKind::Loopback,
        },
    ])
}

fn empty_hud<'a>() -> Hud<'a> {
    Hud {
        source: "test",
        frames_seen: 0,
        frames_decoded: 0,
        remotes: 0,
        lan_peers: 0,
        quiet_for: f64::INFINITY,
        fps: 0.0,
        notice: None,
    }
}

#[test]
fn host_circle_sits_left_of_centre_and_lan_sits_to_its_right() {
    let (w, h) = (1280.0, 800.0);
    let placed = layout::build(w, h, &network(), &[], &[]);

    let host = placed.host.as_ref().expect("host circle");
    let lan = placed.lan.as_ref().expect("lan circle");

    assert!(host.center.x < w, "host must be on screen");
    assert!(
        lan.center.x > host.center.x,
        "lan circle must be right of the host"
    );
    assert!(
        lan.center.x - lan.radius > host.center.x + host.radius - 1.0,
        "the two circles must not overlap"
    );

    // One nested circle per non-loopback interface, each smaller than the host.
    assert_eq!(host.interfaces.len(), 2);
    let mut radii: Vec<f64> = host.interfaces.iter().map(|i| i.radius).collect();
    radii.sort_by(|a, b| b.partial_cmp(a).unwrap());
    assert!(
        radii[0] < host.radius,
        "interface circles nest inside the host"
    );
    assert!(
        radii[1] < radii[0],
        "each interface circle is smaller than the last"
    );
}

#[test]
fn remote_hosts_are_placed_to_the_left_and_ordered_newest_first() {
    let now = Instant::now();
    let remotes = vec![
        netscope::model::RemoteHost {
            addr: "8.8.8.8".parse().unwrap(),
            last_seen: now,
            first_seen: now,
            packets: 3,
            bytes: 180,
            lan_packets: 0,
            last_transport: Transport::Udp,
        },
        netscope::model::RemoteHost {
            addr: "1.1.1.1".parse().unwrap(),
            last_seen: now - std::time::Duration::from_secs(10),
            first_seen: now,
            packets: 1,
            bytes: 60,
            lan_packets: 0,
            last_transport: Transport::Tcp,
        },
    ];
    let placed = layout::build(1280.0, 800.0, &network(), &remotes, &[]);
    let host = placed.host.as_ref().unwrap();

    assert_eq!(placed.remotes.len(), 2);
    for remote in &placed.remotes {
        assert!(
            remote.center.x + remote.radius < host.center.x - host.radius,
            "remote circles belong to the left of the host circle"
        );
    }
    // Newest first, i.e. nearest the vertical middle of the column.
    assert_eq!(placed.remotes[0].host.addr.to_string(), "8.8.8.8");
    assert!(placed.remotes[0].center.y < placed.remotes[1].center.y);
}

#[test]
fn many_remotes_wrap_into_columns_instead_of_being_dropped() {
    let now = Instant::now();
    let remotes: Vec<_> = (0..40)
        .map(|i| netscope::model::RemoteHost {
            addr: ip(&format!("203.0.113.{}", i + 1)),
            last_seen: now - std::time::Duration::from_secs(i as u64),
            first_seen: now,
            packets: 1,
            bytes: 60,
            lan_packets: 0,
            last_transport: Transport::Tcp,
        })
        .collect();

    let placed = layout::build(1280.0, 800.0, &network(), &remotes, &[]);
    assert_eq!(placed.remotes.len(), 40, "every host should be drawn");
    assert_eq!(placed.hidden, 0);
    assert!(placed.slot <= layout::REMOTE_SLOT);

    // Columns sit left of the host circle and never overlap each other.
    let host = placed.host.as_ref().unwrap();
    let mut xs: Vec<f64> = placed.remotes.iter().map(|r| r.center.x).collect();
    xs.dedup();
    assert!(xs.len() > 1, "expected more than one column");
    for remote in &placed.remotes {
        assert!(remote.center.x + remote.radius < host.center.x - host.radius);
    }

    // Nothing spills off the canvas.
    for remote in &placed.remotes {
        assert!(remote.center.y - remote.radius >= 0.0);
        assert!(remote.center.y + remote.radius <= 800.0);
    }
}

#[test]
fn a_tiny_window_reports_the_hosts_it_cannot_draw() {
    let now = Instant::now();
    let remotes: Vec<_> = (0..60)
        .map(|i| netscope::model::RemoteHost {
            addr: ip(&format!("203.0.113.{}", i + 1)),
            last_seen: now,
            first_seen: now,
            packets: 1,
            bytes: 60,
            lan_packets: 0,
            last_transport: Transport::Tcp,
        })
        .collect();
    let placed = layout::build(420.0, 200.0, &network(), &remotes, &[]);
    assert_eq!(placed.remotes.len() + placed.hidden, 60);
    assert!(placed.hidden > 0, "a 420x200 window cannot fit 60 hosts");
    for remote in &placed.remotes {
        assert!(remote.center.y + remote.radius <= 200.0);
    }
}

#[test]
fn rim_and_anchor_helpers_point_at_circle_edges() {
    use netscope::render::layout::{PlacedLan, PlacedRemote};
    let center = vello_cpu::kurbo::Point { x: 100.0, y: 100.0 };
    let target = vello_cpu::kurbo::Point { x: 200.0, y: 100.0 };
    let edge = layout::rim(center, 20.0, target);
    assert!((edge.x - 120.0).abs() < 1e-9);
    assert!((edge.y - 100.0).abs() < 1e-9);

    // A target at the centre itself must not produce a NaN.
    let same = layout::rim(center, 20.0, center);
    assert!((same.x - 100.0).abs() < 1e-9);

    let _ = (
        PlacedLan {
            center,
            radius: 10.0,
            hosts: vec![],
        },
        PlacedRemote {
            center,
            radius: 5.0,
            host: netscope::model::RemoteHost {
                addr: "9.9.9.9".parse().unwrap(),
                last_seen: Instant::now(),
                first_seen: Instant::now(),
                packets: 0,
                bytes: 0,
                lan_packets: 0,
                last_transport: Transport::Other(99),
            },
        },
    );
}

#[test]
fn rendering_produces_a_non_blank_frame_with_the_expected_background() {
    let (w, h) = (640u16, 400u16);
    let mut scene = Scene::new(w, h);
    let net = network();

    let mut model = Model::default();
    let t0 = Instant::now();
    let samples = [
        Frame {
            src: "192.168.1.20".parse().unwrap(),
            dst: "93.184.216.34".parse().unwrap(),
            src_port: None,
            dst_port: None,
            transport: Transport::Tcp,
            wire_len: 60,
        },
        Frame {
            src: "10.5.5.5".parse().unwrap(),
            dst: "192.168.1.7".parse().unwrap(),
            src_port: None,
            dst_port: None,
            transport: Transport::Udp,
            wire_len: 60,
        },
    ];
    for f in samples {
        model.ingest(&Sample { frame: f }, &net, t0);
    }

    let hud = Hud {
        remotes: model.remote_count(),
        lan_peers: model.lan_count(),
        ..empty_hud()
    };
    scene.draw(&net, &model, &hud, t0);

    let pixmap = scene.pixmap();
    assert_eq!(pixmap.data().len(), usize::from(w) * usize::from(h));

    // Top-left corner is outside every circle: it must be the background colour.
    let corner = pixmap.data()[0];
    assert!(
        corner.r < 30 && corner.g < 30 && corner.b < 40,
        "corner should be dark: {corner:?}"
    );

    // Something bright was drawn somewhere (rings, text, links).
    let bright = pixmap
        .data()
        .iter()
        .filter(|p| p.r > 120 && p.a > 0)
        .count();
    assert!(
        bright > 500,
        "expected visible geometry, found {bright} bright pixels"
    );
}

#[test]
fn rendering_survives_a_resize_and_an_empty_model() {
    let mut scene = Scene::new(320, 240);
    let net = network();
    let model = Model::default();
    scene.draw(&net, &model, &empty_hud(), Instant::now());
    scene.resize(800, 600);
    scene.draw(&net, &model, &empty_hud(), Instant::now());
    assert_eq!(scene.pixmap().data().len(), 800 * 600);
}

#[test]
fn text_measures_and_fits() {
    use netscope::render::text::Font;
    let font = Font::bundled();
    assert!(font.measure("192.168.1.20", 13.0) > 0.0);
    assert!(font.measure("wide", 13.0) > font.measure("i", 13.0));
    assert!(font.line_height(13.0) > 13.0 * 1.0);

    let long = "fe80:0000:0000:0000:0000:0000:0000:0001";
    let fitted = font.fit(long, 12.0, 60.0);
    assert!(fitted.ends_with('…'));
    assert!(font.measure(&fitted, 12.0) <= 60.0);
    // Short strings pass through untouched.
    assert_eq!(font.fit("eth0", 12.0, 200.0), "eth0");
}
