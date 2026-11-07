//! Layout placement and an end-to-end Vello CPU render.

use netscope::capture::Sample;
use netscope::capture::parse::{Frame, Transport};
use netscope::model::{LanHost, Model};
use netscope::net::{AddressKind, LocalAddress, LocalNetwork};
use netscope::render::{Hud, Scene, layout};
use netscope::service::Service;
use std::time::{Duration, Instant};
use vello_cpu::kurbo::Point;

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

fn remote(addr: &str, last_seen: Instant, packets: u64) -> netscope::model::RemoteHost {
    netscope::model::RemoteHost {
        addr: ip(addr),
        last_seen,
        first_seen: last_seen,
        packets,
        bytes: packets * 60,
        lan_packets: 0,
        last_transport: Transport::Tcp,
    }
}

fn lan_peer(addr: &str, last_seen: Instant) -> LanHost {
    LanHost {
        addr: ip(addr),
        last_seen,
        packets: 1,
        bytes: 60,
    }
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
fn host_panel_is_a_tall_rectangle_in_the_upper_right() {
    let (w, h) = (1280.0, 800.0);
    let placed = layout::build(w, h, &network(), &[], &[]);
    let host = placed.host.as_ref().expect("host panel");
    let rect = host.rect;

    assert!(rect.x0 > w * 0.5, "host panel belongs on the right half");
    assert!(rect.y0 < h * 0.5, "host panel belongs in the upper half");
    assert!(
        rect.height() > rect.width(),
        "the panel is a vertical rectangle"
    );
    assert!(rect.x1 <= w && rect.y1 <= h, "panel stays on screen");
}

#[test]
fn every_local_address_gets_a_square_inside_the_panel() {
    let placed = layout::build(1280.0, 800.0, &network(), &[], &[]);
    let host = placed.host.as_ref().unwrap();

    // Loopback included: one square per address assigned to the host.
    assert_eq!(host.interfaces.len(), 3);
    for iface in &host.interfaces {
        let square = iface.rect;
        assert!(
            square.x0 >= host.rect.x0 && square.x1 <= host.rect.x1,
            "square for {} inside the panel horizontally",
            iface.addr
        );
        assert!(
            square.y0 >= host.rect.y0 && square.y1 <= host.rect.y1,
            "square for {} inside the panel vertically",
            iface.addr
        );
        assert!(
            (square.width() - square.height()).abs() < 0.01,
            "the shape for {} is a square",
            iface.addr
        );
    }

    // Squares stack top to bottom without overlapping.
    let mut ys: Vec<(f64, f64)> = host
        .interfaces
        .iter()
        .map(|i| (i.rect.y0, i.rect.y1))
        .collect();
    ys.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    for pair in ys.windows(2) {
        assert!(pair[0].1 < pair[1].0, "stacked squares must not overlap");
    }
}

#[test]
fn lan_circle_sits_in_the_lower_right_below_the_host_panel() {
    let (w, h) = (1280.0, 800.0);
    let placed = layout::build(w, h, &network(), &[], &[]);
    let host = placed.host.as_ref().unwrap();
    let lan = placed.lan.as_ref().expect("lan circle");

    assert!(lan.center.y > h * 0.5, "lan circle in the lower half");
    assert!(lan.center.x > w * 0.5, "lan circle on the right");
    assert!(
        lan.center.y - lan.radius > host.rect.y1,
        "lan circle must sit below the host panel"
    );
    assert!(
        lan.radius >= layout::LAN_MIN_RADIUS,
        "the lan circle is large enough to hold peers"
    );
    assert!(
        lan.center.y + lan.radius <= h + 0.01 && lan.center.x + lan.radius <= w + 0.01,
        "lan circle stays on screen"
    );
    assert!(
        lan.center.x - lan.radius >= 0.0,
        "lan circle stays on screen"
    );
}

#[test]
fn lan_peers_get_their_own_circles_inside_the_lan_circle() {
    let now = Instant::now();
    let peers: Vec<_> = ["192.168.1.7", "192.168.1.9", "192.168.1.13"]
        .iter()
        .map(|a| lan_peer(a, now))
        .collect();
    let placed = layout::build(1280.0, 800.0, &network(), &[], &peers);
    let lan = placed.lan.as_ref().unwrap();

    assert_eq!(lan.hosts.len(), 3);
    assert_eq!(lan.hidden, 0);
    for peer in &lan.hosts {
        let dist = (peer.offset.x * peer.offset.x + peer.offset.y * peer.offset.y).sqrt();
        assert!(
            dist + peer.radius <= lan.radius,
            "peer {} must fit inside the lan circle",
            peer.addr
        );
    }

    // Distinct addresses, distinct circles.
    let mut addrs: Vec<_> = lan.hosts.iter().map(|p| p.addr).collect();
    addrs.sort();
    addrs.dedup();
    assert_eq!(addrs.len(), 3);
}

#[test]
fn many_lan_peers_shrink_before_being_dropped() {
    let now = Instant::now();
    let peers: Vec<_> = (0..40)
        .map(|i| lan_peer(&format!("192.168.1.{}", i + 2), now))
        .collect();
    let placed = layout::build(1280.0, 800.0, &network(), &[], &peers);
    let lan = placed.lan.as_ref().unwrap();

    assert_eq!(lan.hosts.len() + lan.hidden, 40);
    assert!(lan.hosts.len() > 10, "a big circle should hold most peers");
    for peer in &lan.hosts {
        let dist = (peer.offset.x * peer.offset.x + peer.offset.y * peer.offset.y).sqrt();
        assert!(dist + peer.radius <= lan.radius);
    }
}

#[test]
fn remote_hosts_are_placed_to_the_left_of_both_right_hand_shapes() {
    let now = Instant::now();
    let remotes = vec![
        remote("8.8.8.8", now, 3),
        remote("1.1.1.1", now - std::time::Duration::from_secs(10), 1),
    ];
    let placed = layout::build(1280.0, 800.0, &network(), &remotes, &[]);
    let host = placed.host.as_ref().unwrap();
    let lan = placed.lan.as_ref().unwrap();

    assert_eq!(placed.remotes.len(), 2);
    for r in &placed.remotes {
        assert!(
            r.center.x + r.radius < host.rect.x0,
            "remotes left of the panel"
        );
        assert!(
            r.center.x + r.radius < lan.center.x + lan.radius,
            "remotes left of the lan circle"
        );
    }

    // Newest first, so the busiest hosts sit at the top.
    assert_eq!(placed.remotes[0].host.addr.to_string(), "8.8.8.8");
    assert!(placed.remotes[0].center.y < placed.remotes[1].center.y);
}

#[test]
fn many_remotes_wrap_into_columns_instead_of_being_dropped() {
    let now = Instant::now();
    let remotes: Vec<_> = (0..40)
        .map(|i| {
            remote(
                &format!("203.0.113.{}", i + 1),
                now - std::time::Duration::from_secs(i as u64),
                1,
            )
        })
        .collect();

    let placed = layout::build(1280.0, 800.0, &network(), &remotes, &[]);
    assert_eq!(placed.remotes.len(), 40, "every host should be drawn");
    assert_eq!(placed.hidden, 0);
    assert!(placed.slot <= layout::REMOTE_SLOT);

    let host = placed.host.as_ref().unwrap();
    let mut xs: Vec<f64> = placed.remotes.iter().map(|r| r.center.x).collect();
    xs.dedup();
    assert!(xs.len() > 1, "expected more than one column");
    for r in &placed.remotes {
        assert!(r.center.x + r.radius < host.rect.x0);
        assert!(r.center.y - r.radius >= 0.0);
        assert!(r.center.y + r.radius <= 800.0);
    }
}

#[test]
fn a_tiny_window_reports_the_hosts_it_cannot_draw() {
    let now = Instant::now();
    let remotes: Vec<_> = (0..60)
        .map(|i| remote(&format!("203.0.113.{}", i + 1), now, 1))
        .collect();
    let placed = layout::build(420.0, 200.0, &network(), &remotes, &[]);
    assert_eq!(placed.remotes.len() + placed.hidden, 60);
    assert!(placed.hidden > 0, "a 420x200 window cannot fit 60 hosts");
    for r in &placed.remotes {
        assert!(r.center.y + r.radius <= 200.0);
    }
}

#[test]
fn link_anchors_land_on_the_right_shape() {
    let placed = layout::build(1280.0, 800.0, &network(), &[], &[]);
    let host = placed.host.as_ref().unwrap();
    let lan = placed.lan.as_ref().unwrap();
    let remote_center = Point { x: 120.0, y: 400.0 };

    // Each interface anchors on its own square, not the panel as a whole.
    let a = layout::local_anchor(&placed, ip("192.168.1.20"), remote_center).unwrap();
    let b = layout::local_anchor(&placed, ip("127.0.0.1"), remote_center).unwrap();
    assert_ne!(a, b, "different addresses anchor on different squares");
    for anchor in [a, b] {
        assert!(
            anchor.x >= host.rect.x0 - 0.01 && anchor.x <= host.rect.x1 + 0.01,
            "interface anchor on the panel"
        );
    }

    // An address with no square of its own falls back to the panel edge.
    let missing = layout::local_anchor(&placed, ip("10.0.0.1"), remote_center).unwrap();
    assert!(missing.x >= host.rect.x0 - 0.01 && missing.x <= host.rect.x1 + 0.01);

    // Host <-> lan links anchor on the panel edge and the lan rim.
    let from_host = layout::host_anchor(&placed, lan.center).unwrap();
    let from_lan = layout::lan_anchor(&placed, layout::rect_center(host.rect)).unwrap();
    assert!(from_host.x >= host.rect.x0 - 0.01 && from_host.x <= host.rect.x1 + 0.01);
    let d = ((from_lan.x - lan.center.x).powi(2) + (from_lan.y - lan.center.y).powi(2)).sqrt();
    assert!((d - lan.radius).abs() < 0.01, "lan anchor sits on the rim");
}

#[test]
fn each_interface_links_to_its_own_square() {
    let placed = layout::build(1280.0, 800.0, &network(), &[], &[]);
    let host = placed.host.as_ref().unwrap();
    let target = Point { x: 120.0, y: 400.0 };

    for iface in &host.interfaces {
        let anchor = layout::local_anchor(&placed, iface.addr, target).unwrap();
        let square = iface.rect;
        // The anchor must sit on the square's own edge, not merely inside the
        // panel: it is on the square boundary and level with that row only.
        assert!(
            anchor.x >= square.x0 - 0.01 && anchor.x <= square.x1 + 0.01,
            "anchor for {} on its square's x span",
            iface.addr
        );
        assert!(
            anchor.y >= square.y0 - 0.01 && anchor.y <= square.y1 + 0.01,
            "anchor for {} on its square's y span",
            iface.addr
        );
    }
}

#[test]
fn remote_columns_start_below_the_hud_text() {
    let now = Instant::now();
    let remotes: Vec<_> = (0..6)
        .map(|i| remote(&format!("203.0.113.{}", i + 1), now, 1))
        .collect();
    let placed = layout::build(1280.0, 800.0, &network(), &remotes, &[]);
    for r in &placed.remotes {
        assert!(
            r.center.y - r.radius >= layout::HUD_RESERVE,
            "remote labels must not collide with the HUD"
        );
    }
}

#[test]
fn rim_and_rect_edge_helpers_point_at_shape_edges() {
    let center = Point { x: 100.0, y: 100.0 };
    let target = Point { x: 300.0, y: 100.0 };

    let edge = layout::rim(center, 20.0, target);
    assert!((edge.x - 120.0).abs() < 1e-9);
    assert!((edge.y - 100.0).abs() < 1e-9);

    let same = layout::rim(center, 20.0, center);
    assert_eq!(same, center);

    // A ray to the right leaves a 40x20 box at its right edge.
    let rect_edge = layout::rect_edge(center, 20.0, 10.0, target);
    assert!((rect_edge.x - 120.0).abs() < 1e-9);
    assert!((rect_edge.y - 100.0).abs() < 1e-9);

    // A steep ray leaves through the top edge instead.
    let steep = layout::rect_edge(center, 20.0, 10.0, Point { x: 110.0, y: 0.0 });
    assert!((steep.y - 90.0).abs() < 1e-9);
    assert!(steep.x > 100.0 && steep.x < 120.0);
}

/// Measures the `service`-coloured line in a rendered frame.
///
/// Every pixel is projected onto the ray from the background colour to the
/// service colour; `t` is the blend weight along that ray. Returns the brightest
/// such weight found, and how many pixels sit at a weight high enough to be a
/// visible line. Anti-aliased text sits near t~0.07, so the presence threshold
/// separates a real line from incidental tinting.
fn line_signal(model: &Model, at: Instant, service: Service) -> (f64, usize) {
    let mut scene = Scene::new(1280, 800);
    let hud = Hud {
        source: "test",
        frames_seen: 1,
        frames_decoded: 1,
        remotes: model.remote_count(),
        lan_peers: model.lan_count(),
        quiet_for: 0.0,
        fps: 0.0,
        notice: None,
    };
    scene.draw(&network(), model, &hud, at);

    let bg = (0.047f64, 0.063, 0.094);
    let [tr, tg, tb] = service.rgb();
    let (dr, dg, db) = (
        f64::from(tr) - bg.0,
        f64::from(tg) - bg.1,
        f64::from(tb) - bg.2,
    );
    let denom = dr * dr + dg * dg + db * db;

    let mut brightest = 0.0f64;
    let mut present = 0usize;
    for px in scene.pixmap().data() {
        let (r, g, b) = (
            f64::from(px.r) / 255.0 - bg.0,
            f64::from(px.g) / 255.0 - bg.1,
            f64::from(px.b) / 255.0 - bg.2,
        );
        let t = (r * dr + g * dg + b * db) / denom;
        if t <= 0.0 || t > 1.05 {
            continue;
        }
        // Must lie on the BG->service ray, not merely be tinted by it.
        if (r - t * dr).abs() < 0.03 && (g - t * dg).abs() < 0.03 && (b - t * db).abs() < 0.03 {
            brightest = brightest.max(t);
            if t > 0.3 {
                present += 1;
            }
        }
    }
    (brightest, present)
}

fn https() -> Service {
    Service::new(Transport::Tcp, Some(443))
}

fn model_with_service(transport: Transport, port: Option<u16>, at: Instant) -> Model {
    let mut model = Model::default();
    model.ingest(
        &Sample {
            frame: Frame {
                src: ip("192.168.1.20"),
                dst: ip("93.184.216.34"),
                src_port: Some(51_000),
                dst_port: port,
                transport,
                wire_len: 60,
            },
        },
        &network(),
        at,
    );
    model
}

#[test]
fn a_fresh_line_is_drawn_at_full_strength() {
    let now = Instant::now();
    let model = model_with_service(Transport::Tcp, Some(443), now);
    let (brightest, present) = line_signal(&model, now, https());
    assert!(present > 200, "expected a drawn line, got {present} pixels");
    // Drawn at 0.85 alpha, so the blend weight should sit just under that.
    assert!(
        brightest > 0.7,
        "fresh line should be near full brightness, got {brightest}"
    );
}

#[test]
fn a_line_dims_continuously_and_is_gone_after_45s() {
    let t0 = Instant::now();
    let mut model = model_with_service(Transport::Tcp, Some(443), t0);

    let fresh = line_signal(&model, t0, https()).0;

    // 15s in: still visible, but measurably dimmer. Fading starts immediately,
    // so there is no hold period.
    let at15 = t0 + Duration::from_secs(15);
    let (mid, mid_pixels) = line_signal(&model, at15, https());
    assert!(
        mid < fresh - 0.15,
        "line should dim within 15s: {fresh} -> {mid}"
    );
    assert!(mid > 0.3, "line should still be visible at 15s, got {mid}");
    assert!(mid_pixels > 200, "line should still be drawn at 15s");

    // 40s in: too faint to count as present.
    let at40 = t0 + Duration::from_secs(40);
    let (late, late_pixels) = line_signal(&model, at40, https());
    assert!(late < 0.2, "line should be faint at 40s, got {late}");
    assert_eq!(late_pixels, 0, "line should no longer be visible at 40s");

    // Past 45s the flow is pruned and the frame is indistinguishable from one
    // with no traffic at all, even though the circle outlived the line.
    model.prune(t0 + Duration::from_secs(50));
    let (gone, gone_pixels) = line_signal(&model, t0 + Duration::from_secs(50), https());
    let baseline = line_signal(&Model::default(), t0 + Duration::from_secs(50), https()).0;
    assert_eq!(gone_pixels, 0, "line should have faded out completely");
    assert!(
        (gone - baseline).abs() < 0.02,
        "pruned frame should match an empty one: {gone} vs {baseline}"
    );
}

#[test]
fn a_further_packet_refreshes_the_line_to_full_brightness() {
    let t0 = Instant::now();
    let mut model = model_with_service(Transport::Tcp, Some(443), t0);

    let at = t0 + Duration::from_secs(40);
    let (faded, faded_pixels) = line_signal(&model, at, https());
    assert!(faded < 0.2, "should be nearly gone after 40s, got {faded}");
    assert_eq!(
        faded_pixels, 0,
        "line should be invisible before the refresh"
    );

    model.ingest(
        &Sample {
            frame: Frame {
                src: ip("192.168.1.20"),
                dst: ip("93.184.216.34"),
                src_port: Some(51_000),
                dst_port: Some(443),
                transport: Transport::Tcp,
                wire_len: 60,
            },
        },
        &network(),
        at,
    );
    let (refreshed, refreshed_pixels) = line_signal(&model, at, https());
    assert!(
        refreshed > 0.7,
        "packet should refresh the line to full brightness, got {refreshed}"
    );
    assert!(refreshed_pixels > 200, "refreshed line should be redrawn");
}

#[test]
fn parallel_connections_to_one_host_are_all_visible() {
    // Three services to the same remote share endpoints, so drawing them on top
    // of each other would hide all but one. They must be fanned apart.
    let now = Instant::now();
    let mut model = Model::default();
    for port in [443u16, 80, 22] {
        model.ingest(
            &Sample {
                frame: Frame {
                    src: ip("192.168.1.20"),
                    dst: ip("93.184.216.34"),
                    src_port: Some(51_000),
                    dst_port: Some(port),
                    transport: Transport::Tcp,
                    wire_len: 60,
                },
            },
            &network(),
            now,
        );
    }
    assert_eq!(model.flows().count(), 3);

    for port in [443u16, 80, 22] {
        let service = Service::new(Transport::Tcp, Some(port));
        let (brightest, present) = line_signal(&model, now, service);
        assert!(
            present > 200,
            "tcp/{port} is hidden behind the other lines ({present} visible pixels)"
        );
        assert!(brightest > 0.7, "tcp/{port} line dim: {brightest}");
    }
}

#[test]
fn each_service_is_rendered_in_its_own_colour() {
    // Each service's line must be found at its own colour and not at another's.
    let now = Instant::now();
    for service in [
        Service::new(Transport::Icmp, None),
        Service::new(Transport::Udp, Some(53)),
        Service::new(Transport::Tcp, Some(443)),
        Service::new(Transport::Tcp, Some(80)),
    ] {
        let model = model_with_service(service.transport, service.port, now);
        let (brightest, present) = line_signal(&model, now, service);
        assert!(present > 200, "{} line missing", service.label());
        assert!(brightest > 0.7, "{} line dim", service.label());
    }
}

#[test]
fn rendering_produces_a_non_blank_frame_with_the_expected_background() {
    let mut scene = Scene::new(640, 480);
    let model = Model::default();
    let placed = scene.draw(&network(), &model, &empty_hud(), Instant::now());

    let data = scene.pixmap().data();
    assert_eq!(data.len(), 640 * 480);
    assert_eq!(placed.remotes.len(), 0);

    // A corner pixel should be the background colour, not black or garbage.
    let corner = data[0];
    assert!(corner.r < 60 && corner.g < 60 && corner.b < 90);
    assert_eq!(corner.a, 255);
}

#[test]
fn rendering_a_full_scene_draws_panel_lan_and_remotes() {
    let now = Instant::now();
    let mut model = Model::default();
    let net = network();
    // host -> remote, remote -> lan peer, lan peer -> host interface
    for (src, dst) in [
        ("192.168.1.20", "8.8.8.8"),
        ("1.1.1.1", "192.168.1.7"),
        ("192.168.1.9", "192.168.1.20"),
    ] {
        let frame = Frame {
            src: ip(src),
            dst: ip(dst),
            src_port: None,
            dst_port: None,
            transport: Transport::Tcp,
            wire_len: 60,
        };
        model.ingest(&Sample { frame }, &net, now);
    }

    let mut scene = Scene::new(1280, 800);
    let placed = scene.draw(&net, &model, &empty_hud(), now);
    assert_eq!(placed.remotes.len(), 2);
    assert!(!placed.lan.as_ref().unwrap().hosts.is_empty());
    assert_eq!(placed.host.as_ref().unwrap().interfaces.len(), 3);
    assert_eq!(scene.pixmap().data().len(), 1280 * 800);
}

#[test]
fn rendering_survives_a_resize_and_an_empty_model() {
    let mut scene = Scene::new(1280, 800);
    scene.resize(320, 240);
    let placed = scene.draw(&network(), &Model::default(), &empty_hud(), Instant::now());
    assert!(placed.host.is_some());
    assert!(placed.lan.is_some());
    assert_eq!(scene.pixmap().data().len(), 320 * 240);
}
