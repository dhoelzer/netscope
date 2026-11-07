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
    remote_hops(addr, last_seen, packets, None)
}

fn remote_hops(
    addr: &str,
    last_seen: Instant,
    packets: u64,
    hops: Option<u32>,
) -> netscope::model::RemoteHost {
    remote_group(addr, last_seen, packets, hops, false)
}

fn remote_group(
    addr: &str,
    last_seen: Instant,
    packets: u64,
    hops: Option<u32>,
    group: bool,
) -> netscope::model::RemoteHost {
    netscope::model::RemoteHost {
        addr: ip(addr),
        last_seen,
        first_seen: last_seen,
        packets,
        bytes: packets * 60,
        lan_packets: 0,
        last_transport: Transport::Tcp,
        hops,
        hops_learned_at: None,
        group,
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

/// Clock for layout tests: no test animates a slide, so any instant works.
fn t0() -> Instant {
    Instant::now()
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
    let placed = layout::build(w, h, &network(), &[], &[], t0());
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
    let placed = layout::build(1280.0, 800.0, &network(), &[], &[], t0());
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
    let placed = layout::build(w, h, &network(), &[], &[], t0());
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
    let placed = layout::build(1280.0, 800.0, &network(), &[], &peers, t0());
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
    let placed = layout::build(1280.0, 800.0, &network(), &[], &peers, t0());
    let lan = placed.lan.as_ref().unwrap();

    assert_eq!(lan.hosts.len() + lan.hidden, 40);
    assert!(lan.hosts.len() > 10, "a big circle should hold most peers");
    for peer in &lan.hosts {
        let dist = (peer.offset.x * peer.offset.x + peer.offset.y * peer.offset.y).sqrt();
        assert!(dist + peer.radius <= lan.radius);
    }
}

#[test]
fn remote_circles_never_land_on_the_host_panel_or_lan_circle() {
    let now = Instant::now();
    // Spread across every guide line, including the ones that run beside and
    // through the right-hand shapes.
    let remotes: Vec<_> = (1..=15)
        .map(|h| remote_hops(&format!("203.0.113.{h}"), now, 1, Some(h)))
        .collect();
    let placed = layout::build(1280.0, 800.0, &network(), &remotes, &[], t0());
    let host = placed.host.as_ref().unwrap();
    let lan = placed.lan.as_ref().unwrap();

    assert_eq!(placed.remotes.len(), 15);
    for r in &placed.remotes {
        let (x, y, rad) = (r.center.x, r.center.y, r.radius);
        let clear_panel = x + rad < host.rect.x0
            || x - rad > host.rect.x1
            || y + rad < host.rect.y0
            || y - rad > host.rect.y1;
        assert!(
            clear_panel,
            "remote on {} hops overlaps the host panel at ({x:.0},{y:.0})",
            r.host.hops.unwrap()
        );
        let dist = ((x - lan.center.x).powi(2) + (y - lan.center.y).powi(2)).sqrt();
        assert!(
            dist >= lan.radius - 0.01,
            "remote on {} hops overlaps the lan circle at ({x:.0},{y:.0})",
            r.host.hops.unwrap()
        );
        assert!(y - rad >= 0.0 && y + rad <= 800.0, "stays on screen");
    }
}

#[test]
fn remotes_on_one_line_stack_newest_first() {
    let now = Instant::now();
    let remotes: Vec<_> = (0..3)
        .map(|i| {
            remote_hops(
                &format!("203.0.113.{}", i + 1),
                now - std::time::Duration::from_secs(i * 10),
                1,
                Some(6),
            )
        })
        .collect();
    let placed = layout::build(1280.0, 800.0, &network(), &remotes, &[], t0());

    let line = layout::hop_line_positions(1280.0)[5];
    let on_line: Vec<_> = placed
        .remotes
        .iter()
        .filter(|r| (r.center.x - line).abs() < 0.01)
        .collect();
    assert_eq!(on_line.len(), 3, "all three share the 6-hop line");
    assert_eq!(on_line[0].host.addr.to_string(), "203.0.113.1");
    for pair in on_line.windows(2) {
        assert!(pair[0].center.y < pair[1].center.y, "newest sits highest");
    }
}

#[test]
fn many_remotes_spread_over_lines_without_being_dropped() {
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

    let remotes: Vec<_> = remotes
        .iter()
        .enumerate()
        .map(|(i, r)| {
            remote_hops(
                &r.addr.to_string(),
                r.last_seen,
                r.packets,
                Some((i as u32 % 15) + 1),
            )
        })
        .collect();
    let placed = layout::build(1280.0, 800.0, &network(), &remotes, &[], t0());
    assert_eq!(placed.remotes.len(), 40, "every host should be drawn");
    assert_eq!(placed.hidden, 0);
    assert!(placed.slot <= layout::REMOTE_SLOT);

    let host = placed.host.as_ref().unwrap();
    let mut xs: Vec<f64> = placed.remotes.iter().map(|r| r.center.x).collect();
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    xs.dedup_by(|a, b| (*a - *b).abs() < 0.01);
    assert!(xs.len() > 1, "expected more than one guide line");
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
    let placed = layout::build(420.0, 200.0, &network(), &remotes, &[], t0());
    assert_eq!(placed.remotes.len() + placed.hidden, 60);
    assert!(placed.hidden > 0, "a 420x200 window cannot fit 60 hosts");
    for r in &placed.remotes {
        assert!(r.center.y + r.radius <= 200.0);
    }
}

#[test]
fn link_anchors_land_on_the_right_shape() {
    let placed = layout::build(1280.0, 800.0, &network(), &[], &[], t0());
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
    let placed = layout::build(1280.0, 800.0, &network(), &[], &[], t0());
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
    let placed = layout::build(1280.0, 800.0, &network(), &remotes, &[], t0());
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
    scene.draw(&network(), model, &hud, at, None);

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
                ttl: 64,
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
                ttl: 64,
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
                    ttl: 64,
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
    let placed = scene.draw(&network(), &model, &empty_hud(), Instant::now(), None);

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
            ttl: 64,
        };
        model.ingest(&Sample { frame }, &net, now);
    }

    let mut scene = Scene::new(1280, 800);
    let placed = scene.draw(&net, &model, &empty_hud(), now, None);
    assert_eq!(placed.remotes.len(), 2);
    assert!(!placed.lan.as_ref().unwrap().hosts.is_empty());
    assert_eq!(placed.host.as_ref().unwrap().interfaces.len(), 3);
    assert_eq!(scene.pixmap().data().len(), 1280 * 800);
}

#[test]
fn rendering_survives_a_resize_and_an_empty_model() {
    let mut scene = Scene::new(1280, 800);
    scene.resize(320, 240);
    let placed = scene.draw(
        &network(),
        &Model::default(),
        &empty_hud(),
        Instant::now(),
        None,
    );
    assert!(placed.host.is_some());
    assert!(placed.lan.is_some());
    assert_eq!(scene.pixmap().data().len(), 320 * 240);
}

// ---- hop guide lines ------------------------------------------------------

/// True when a pixel is visibly different from the scene background.
///
/// Guide-line and circle colours go through vello's float-to-sRGB8 conversion,
/// so the exact bytes are not worth restating here; what matters is that
/// something was drawn where the layout says it should be.
fn off_background(px: &(u8, u8, u8)) -> bool {
    let bg = (12u8, 16, 24);
    [px.0, px.1, px.2]
        .iter()
        .zip([bg.0, bg.1, bg.2])
        .any(|(got, w)| (*got as i16 - w as i16).abs() > 12)
}

#[test]
fn all_twenty_guide_lines_are_drawn_where_the_layout_says() {
    let now = Instant::now();
    let mut scene = Scene::new(1280, 800);
    scene.draw(&network(), &Model::default(), &empty_hud(), now, None);

    let data = scene.pixmap().data();
    let width = 1280usize;
    let positions = layout::hop_line_positions(1280.0);
    assert_eq!(positions.len(), layout::HOP_LINES);
    assert_eq!(layout::HOP_LINES, 20);

    for (i, x) in positions.iter().enumerate() {
        let col = (*x as usize).clamp(1, width - 2);
        // Count drawn pixels in a band above the LAN circle.
        let lit = (100usize..380)
            .filter(|y| {
                off_background(&(
                    data[y * width + col].r,
                    data[y * width + col].g,
                    data[y * width + col].b,
                ))
            })
            .count();
        assert!(
            lit > 100,
            "guide line {i} at x={x:.0} has only {lit} drawn pixels"
        );

        // Halfway to the next line must be empty, or we are measuring a band
        // of background rather than a line.
        if let Some(next) = positions.get(i + 1) {
            let between = ((x + next) / 2.0) as usize;
            let stray = (100usize..380)
                .filter(|y| {
                    off_background(&(
                        data[y * width + between].r,
                        data[y * width + between].g,
                        data[y * width + between].b,
                    ))
                })
                .count();
            assert_eq!(stray, 0, "something drawn between lines {i} and {}", i + 1);
        }
    }
}

#[test]
fn guide_lines_are_dashed_not_solid() {
    let now = Instant::now();
    let mut scene = Scene::new(1280, 800);
    scene.draw(&network(), &Model::default(), &empty_hud(), now, None);

    let data = scene.pixmap().data();
    let width = 1280usize;
    let x = layout::hop_line_positions(1280.0)[7] as usize;

    let mut runs = 0;
    let mut inside = false;
    for y in 100usize..380 {
        let px = &data[y * width + x];
        let lit = off_background(&(px.r, px.g, px.b));
        if lit && !inside {
            runs += 1;
        }
        inside = lit;
    }
    // A solid line would give one run; 5-on/5-off over 280px gives far more.
    assert!(runs > 15, "expected a dashed line, found {runs} run(s)");
}

#[test]
fn a_remote_is_drawn_on_the_line_its_ttl_implied() {
    let now = Instant::now();
    let net = network();
    let mut model = Model::default();
    // TTL 58 arriving from 64 means 6 hops, so this host belongs on line 6.
    let frame = Frame {
        src: ip("203.0.113.40"),
        dst: ip("192.168.1.20"),
        src_port: None,
        dst_port: None,
        transport: Transport::Tcp,
        wire_len: 60,
        ttl: 58,
    };
    model.ingest(&Sample { frame }, &net, now);

    let mut scene = Scene::new(1280, 800);
    let placed = scene.draw(&net, &model, &empty_hud(), now, None);
    assert_eq!(placed.remotes.len(), 1);
    let remote = &placed.remotes[0];
    assert_eq!(remote.host.hops, Some(6));
    assert_eq!(remote.slot, Some(5));

    let expected = layout::hop_line_positions(1280.0)[5];
    assert!(
        (remote.center.x - expected).abs() < 0.01,
        "host should sit on the 6-hop line: {} vs {expected}",
        remote.center.x
    );

    // And something is drawn at that spot: a horizontal scan crossing the
    // circle must hit it.
    let data = scene.pixmap().data();
    let width = 1280usize;
    let (cx, cy) = (remote.center.x as usize, remote.center.y as usize);
    let drawn = (cx - 40..=cx)
        .filter(|x| {
            let px = &data[cy * width + x];
            off_background(&(px.r, px.g, px.b))
        })
        .count();
    assert!(drawn > 5, "nothing drawn on the 6-hop line at y={cy}");
}

#[test]
fn a_host_with_no_ttl_reading_is_parked_at_the_left_edge() {
    let now = Instant::now();
    let net = network();
    let mut model = Model::default();
    // Outbound only: the TTL is ours, so it says nothing about distance.
    let frame = Frame {
        src: ip("192.168.1.20"),
        dst: ip("203.0.113.41"),
        src_port: None,
        dst_port: None,
        transport: Transport::Tcp,
        wire_len: 60,
        ttl: 64,
    };
    model.ingest(&Sample { frame }, &net, now);

    let mut scene = Scene::new(1280, 800);
    let placed = scene.draw(&net, &model, &empty_hud(), now, None);
    let remote = &placed.remotes[0];
    assert_eq!(remote.host.hops, None);
    assert_eq!(remote.slot, None);
    assert!(
        (remote.center.x - placed.far_x).abs() < 0.01,
        "unknown distance belongs at the left edge, got {}",
        remote.center.x
    );
}

#[test]
fn a_distance_learned_later_slides_in_from_the_edge() {
    let net = network();
    let t0 = Instant::now();
    let mut model = Model::default();
    let outbound = Frame {
        src: ip("192.168.1.20"),
        dst: ip("203.0.113.42"),
        src_port: None,
        dst_port: None,
        transport: Transport::Tcp,
        wire_len: 60,
        ttl: 64,
    };
    let inbound = Frame {
        ttl: 60,
        src: ip("203.0.113.42"),
        dst: ip("192.168.1.20"),
        ..outbound
    };
    model.ingest(&Sample { frame: outbound }, &net, t0);
    model.ingest(&Sample { frame: inbound }, &net, t0);

    // Immediately after the reply, the circle is still near the edge.
    let placed = layout::build(
        1280.0,
        800.0,
        &net,
        &model.remotes().cloned().collect::<Vec<_>>(),
        &[],
        t0,
    );
    let early = &placed.remotes[0];
    let line = layout::hop_line_positions(1280.0)[3];
    assert_eq!(early.host.hops, Some(4));
    assert!(
        early.center.x < placed.far_x + (line - placed.far_x) * 0.2,
        "should have barely left the edge, got {}",
        early.center.x
    );

    // Once the slide has run, it is on its line.
    let later = t0 + std::time::Duration::from_millis(1500);
    let placed = layout::build(
        1280.0,
        800.0,
        &net,
        &model.remotes().cloned().collect::<Vec<_>>(),
        &[],
        later,
    );
    let settled = &placed.remotes[0];
    assert!(
        (settled.center.x - line).abs() < 0.01,
        "should have settled on the line, got {}",
        settled.center.x
    );
}

#[test]
fn hop_labels_stay_separate_at_every_window_width() {
    // The numbers sit centred on their lines, so a narrow window squeezes them
    // together; each must still occupy its own run of pixels.
    for width in [1280usize, 900, 620] {
        let mut scene = Scene::new(width as u16, 800);
        scene.draw(
            &network(),
            &Model::default(),
            &empty_hud(),
            Instant::now(),
            None,
        );

        let data = scene.pixmap().data();
        let positions = layout::hop_line_positions(width as f64);
        let mut clusters: Vec<(usize, usize)> = Vec::new();
        for x in 0..width {
            let drawn = (12usize..34).any(|y| {
                let px = &data[y * width + x];
                off_background(&(px.r, px.g, px.b))
            });
            if !drawn {
                continue;
            }
            // Merge across the gap between digits; 5px is far below the
            // narrowest pitch, so it cannot hide a real collision.
            match clusters.last_mut() {
                Some(last) if x - last.1 <= 5 => last.1 = x,
                _ => clusters.push((x, x)),
            }
        }
        // Every line must have a cluster centred on it, and that cluster must
        // not spill into its neighbour's. The axis caption is extra, so the
        // count is checked per line rather than in total.
        let pitch = positions[0] - positions[1];
        for (i, x) in positions.iter().enumerate() {
            let centre = clusters
                .iter()
                .find(|(a, b)| *a <= *x as usize && *x as usize <= *b)
                .unwrap_or_else(|| {
                    panic!("no hop label centred on line {i} (x={x:.0}) at width {width}")
                });
            assert!(
                (centre.1 - centre.0) as f64 <= pitch - 4.0,
                "hop label {centre:?} too wide for a {pitch:.0}px pitch at width {width}"
            );
        }
    }
}

#[test]
fn group_hosts_are_parked_between_the_first_line_and_the_lan_circle() {
    let now = Instant::now();
    let remotes = vec![
        // A group whose TTL would have put it on line 4 if the ruler applied.
        remote_group("224.0.0.251", now, 5, Some(4), true),
        remote_group("255.255.255.255", now, 3, None, true),
        // A unicast host at 4 hops, for contrast.
        remote_hops("203.0.113.40", now, 2, Some(4)),
    ];
    let placed = layout::build(1920.0, 800.0, &network(), &remotes, &[], now);

    let line_one = placed.hop_lines[0].x;
    let lan = placed.lan.as_ref().expect("lan circle");
    let rim = lan.center.x - lan.radius;
    assert!(
        rim > line_one,
        "the layout should leave a lane between the 1-hop line ({line_one}) and the circle rim ({rim})"
    );

    for addr in ["224.0.0.251", "255.255.255.255"] {
        let host = placed
            .remotes
            .iter()
            .find(|r| r.host.addr == ip(addr))
            .unwrap_or_else(|| panic!("{addr} should be placed"));
        assert!(
            host.center.x > line_one && host.center.x < rim,
            "{addr} should sit in the near lane ({line_one}..{rim}), got {}",
            host.center.x
        );
        assert_eq!(host.slot, None, "a group sits on no guide line");
    }

    // The unicast host is unaffected and still sits on its 4-hop line.
    let unicast = placed
        .remotes
        .iter()
        .find(|r| r.host.addr == ip("203.0.113.40"))
        .expect("unicast host");
    assert_eq!(unicast.slot, Some(3));
    assert!((unicast.center.x - placed.hop_lines[3].x).abs() < 0.01);
}

#[test]
fn the_lan_circle_shrinks_to_keep_the_near_lane_open() {
    // A wide window gives the circle room to grow, but it must stop short of the
    // 1-hop line so the group lane stays usable.
    let placed = layout::build(1920.0, 800.0, &network(), &[], &[], t0());
    let lan = placed.lan.as_ref().expect("lan circle");
    let rim = lan.center.x - lan.radius;
    let line_one = placed.hop_lines[0].x;
    assert!(
        rim >= line_one + layout::LAN_LANE_RESERVE - 1.0,
        "rim {rim} should leave {}px of lane right of line 1 at {line_one}",
        layout::LAN_LANE_RESERVE
    );
    assert!(
        placed.near_x > line_one && placed.near_x < rim,
        "the lane centre {} should sit between {line_one} and {rim}",
        placed.near_x
    );
}

// ---- hover / tooltips ----------------------------------------------------

/// A scene with one remote, one LAN peer and traffic in both directions.
fn pkt(src: &str, dst: &str, port: u16) -> Sample {
    Sample {
        frame: Frame {
            src: ip(src),
            dst: ip(dst),
            src_port: Some(51_000),
            dst_port: Some(port),
            transport: if port == 53 {
                Transport::Udp
            } else {
                Transport::Tcp
            },
            wire_len: 60,
            ttl: 64,
        },
    }
}

fn hover_scene() -> (Model, layout::Layout) {
    let net = network();
    let mut model = Model::default();
    let at = Instant::now();
    model.ingest(&pkt("192.168.1.20", "203.0.113.9", 443), &net, at);
    model.ingest(&pkt("203.0.113.9", "192.168.1.20", 443), &net, at);
    model.ingest(&pkt("192.168.1.20", "192.168.1.7", 53), &net, at);
    let placed = scene_layout(&net, &model, at);
    (model, placed)
}

fn scene_layout(net: &LocalNetwork, model: &Model, at: Instant) -> layout::Layout {
    let mut scene = Scene::new(1280, 800);
    scene.draw(net, model, &empty_hud(), at, None)
}

#[test]
fn hovering_a_remote_circle_hits_that_host() {
    let (_model, placed) = hover_scene();
    let remote = placed.remotes.first().expect("a remote circle");
    let hit = layout::hit_test(&placed, remote.center);
    assert_eq!(hit, Some(layout::Hit::Remote(remote.host.addr)));
}

#[test]
fn empty_space_gets_no_tooltip() {
    let (_model, placed) = hover_scene();
    // Far from every shape: the gap between the host panel and the LAN circle
    // is not a line here, and well clear of the remote.
    let hit = layout::hit_test(&placed, Point { x: 700.0, y: 780.0 });
    assert_eq!(hit, None);
}

#[test]
fn hovering_a_traffic_line_hits_the_connection_it_drew() {
    let (_model, placed) = hover_scene();
    let link = placed
        .links
        .iter()
        .find(|l| matches!(l.key, layout::LinkKey::Flow(..)))
        .expect("a flow line should be recorded");
    // The midpoint of the drawn segment is on the line by definition.
    let mid = Point {
        x: (link.a.x + link.b.x) / 2.0,
        y: (link.a.y + link.b.y) / 2.0,
    };
    let hit = layout::hit_test(&placed, mid);
    assert_eq!(hit, Some(layout::Hit::Link(link.key)));
}

#[test]
fn a_line_is_hit_a_few_pixels_off_but_not_far_off() {
    let (_model, placed) = hover_scene();
    let link = placed
        .links
        .iter()
        .find(|l| matches!(l.key, layout::LinkKey::Flow(..)))
        .expect("a flow line should be recorded");
    let mid = Point {
        x: (link.a.x + link.b.x) / 2.0,
        y: (link.a.y + link.b.y) / 2.0,
    };
    // Just inside the tolerance band: still the line.
    let near = layout::hit_test(
        &placed,
        Point {
            x: mid.x,
            y: mid.y + layout::LINK_HIT_TOLERANCE - 1.0,
        },
    );
    assert!(matches!(near, Some(layout::Hit::Link(_))), "got {near:?}");
    // Well outside it: no tooltip, rather than a wrong one.
    let far = layout::hit_test(
        &placed,
        Point {
            x: mid.x,
            y: mid.y + layout::LINK_HIT_TOLERANCE + 40.0,
        },
    );
    assert!(!matches!(far, Some(layout::Hit::Link(_))), "got {far:?}");
}

#[test]
fn a_circle_wins_over_a_line_running_under_it() {
    let (_model, placed) = hover_scene();
    let remote = placed.remotes.first().expect("a remote circle");
    // The link to that host ends at the circle's rim, so sampling just inside
    // the rim must name the host, not the line.
    let inside = Point {
        x: remote.center.x,
        y: remote.center.y + remote.radius * 0.5,
    };
    assert_eq!(
        layout::hit_test(&placed, inside),
        Some(layout::Hit::Remote(remote.host.addr))
    );
}

#[test]
fn hovering_the_host_panel_names_the_host_and_its_interface_wins() {
    let (_model, placed) = hover_scene();
    let host = placed.host.as_ref().expect("host panel");
    let iface = host.interfaces.first().expect("an interface square");
    assert_eq!(
        layout::hit_test(
            &placed,
            Point {
                x: iface.rect.x0 + 2.0,
                y: iface.rect.y0 + 2.0
            }
        ),
        Some(layout::Hit::Interface(iface.addr))
    );
    // A point in the panel but clear of every square is the panel itself.
    let bare = Point {
        x: host.rect.x1 - 3.0,
        y: host.rect.y1 - 3.0,
    };
    assert_eq!(
        layout::hit_test(&placed, bare),
        Some(layout::Hit::HostPanel)
    );
}

#[test]
fn a_lan_peer_dot_wins_over_the_circle_containing_it() {
    let (_model, placed) = hover_scene();
    let lan = placed.lan.as_ref().expect("lan circle");
    let peer = lan.hosts.first().expect("a peer dot");
    let at = Point {
        x: lan.center.x + peer.offset.x,
        y: lan.center.y + peer.offset.y,
    };
    assert_eq!(
        layout::hit_test(&placed, at),
        Some(layout::Hit::LanPeer(peer.addr))
    );
    // Away from any dot but still inside the circle: the circle itself.
    let ring = Point {
        x: lan.center.x,
        y: lan.center.y - lan.radius * 0.85,
    };
    assert_eq!(
        layout::hit_test(&placed, ring),
        Some(layout::Hit::LanCircle)
    );
}

#[test]
fn every_hit_kind_has_an_anchor_for_the_card_to_sit_beside() {
    let (_model, placed) = hover_scene();
    let hits = [
        layout::Hit::Remote(ip("203.0.113.9")),
        layout::Hit::LanPeer(ip("192.168.1.7")),
        layout::Hit::Interface(ip("192.168.1.20")),
        layout::Hit::HostPanel,
        layout::Hit::LanCircle,
    ];
    for hit in hits {
        assert!(
            layout::hit_anchor(&placed, &hit, Point { x: 0.0, y: 0.0 }).is_some(),
            "{hit:?} should have an anchor"
        );
    }
    let key = placed
        .links
        .iter()
        .find(|l| matches!(l.key, layout::LinkKey::Flow(..)))
        .expect("a flow line")
        .key;
    let anchor = layout::hit_anchor(&placed, &layout::Hit::Link(key), Point { x: 0.0, y: 0.0 })
        .expect("line anchor");
    assert!(anchor.width() > 0.0 && anchor.height() > 0.0);
}

#[test]
fn a_line_tooltip_anchors_on_the_pointer_not_the_whole_line() {
    let (_model, placed) = hover_scene();
    let link = placed
        .links
        .iter()
        .find(|l| matches!(l.key, layout::LinkKey::Flow(..)))
        .expect("a flow line");
    let key = link.key;

    // Two points on the same line, far apart along it.
    let near_a = Point {
        x: link.a.x + (link.b.x - link.a.x) * 0.1,
        y: link.a.y + (link.b.y - link.a.y) * 0.1,
    };
    let near_b = Point {
        x: link.a.x + (link.b.x - link.a.x) * 0.9,
        y: link.a.y + (link.b.y - link.a.y) * 0.9,
    };
    let at_a = layout::hit_anchor(&placed, &layout::Hit::Link(key), near_a).expect("anchor at a");
    let at_b = layout::hit_anchor(&placed, &layout::Hit::Link(key), near_b).expect("anchor at b");

    // Each anchor sits on its own pointer, not on a shared box spanning the line.
    for (pointer, anchor) in [(near_a, at_a), (near_b, at_b)] {
        let centre = Point {
            x: (anchor.x0 + anchor.x1) / 2.0,
            y: (anchor.y0 + anchor.y1) / 2.0,
        };
        let off_line = ((pointer.x - centre.x).powi(2) + (pointer.y - centre.y).powi(2)).sqrt();
        assert!(
            off_line <= link.width / 2.0 + 1.0,
            "anchor centre {centre:?} should sit on the line under {pointer:?}, was {off_line} away"
        );
    }
    let moved = ((at_a.x0 - at_b.x0).powi(2) + (at_a.y0 - at_b.y0).powi(2)).sqrt();
    assert!(
        moved > 10.0,
        "the anchor should travel with the pointer, moved only {moved}px"
    );
}

#[test]
fn drawing_with_a_hover_point_renders_a_card_and_without_one_does_not() {
    let (model, _) = hover_scene();
    let net = network();
    let remote_center = {
        let placed = scene_layout(&net, &model, Instant::now());
        placed.remotes.first().expect("a remote").center
    };

    let mut plain = Scene::new(1280, 800);
    plain.draw(&net, &model, &empty_hud(), Instant::now(), None);
    let mut hovered = Scene::new(1280, 800);
    hovered.draw(
        &net,
        &model,
        &empty_hud(),
        Instant::now(),
        Some(remote_center),
    );

    let a = plain.pixmap().data();
    let b = hovered.pixmap().data();
    assert_eq!(a.len(), b.len());
    let changed = a
        .iter()
        .zip(b.iter())
        .step_by(4)
        .filter(|(x, y)| x != y)
        .count();
    // A tooltip card is hundreds of pixels of text and panel; a stray pixel
    // would mean the hover path did something else entirely.
    assert!(
        changed > 500,
        "hovering should draw a card, {changed} px differ"
    );
}

#[test]
fn a_hover_point_outside_every_shape_changes_nothing() {
    let (model, _) = hover_scene();
    let net = network();
    // One timestamp for both draws: line fades depend on it, so two separate
    // Instant::now() calls would legitimately differ in pixel values.
    let now = Instant::now();
    let empty = Point { x: 700.0, y: 780.0 };
    assert_eq!(
        layout::hit_test(&scene_layout(&net, &model, now), empty),
        None
    );
    let mut plain = Scene::new(1280, 800);
    plain.draw(&net, &model, &empty_hud(), now, None);
    let mut hovered = Scene::new(1280, 800);
    hovered.draw(&net, &model, &empty_hud(), now, Some(empty));
    assert_eq!(plain.pixmap().data(), hovered.pixmap().data());
}

// ---- coordinate space of the hover point ---------------------------------

#[test]
fn the_hover_point_is_in_the_same_device_pixels_as_the_layout() {
    // The app sizes the scene from WindowEvent::Resized, which reports a
    // PhysicalSize, so every shape the layout places is in device pixels. The
    // pointer must therefore be passed through unconverted: on a 2x display a
    // logical-point cursor lands at half the coordinates and misses every
    // shape, which is the bug this test pins down.
    let now = Instant::now();
    let mut model = Model::default();
    model.ingest(&pkt("192.168.1.20", "203.0.113.9", 443), &network(), now);

    // A 1920x800-point window on a 2x display: 3840x1600 device pixels.
    let mut scene = Scene::new(3840, 1600);
    let placed = scene.draw(&network(), &model, &empty_hud(), now, None);
    let remote = placed.remotes.first().expect("a remote circle");

    // The pointer sits exactly on the circle, in device pixels.
    let hit = layout::hit_test(&placed, remote.center);
    assert_eq!(hit, Some(layout::Hit::Remote(remote.host.addr)));

    // Halving it, as converting to logical points would, finds nothing.
    let halved = Point {
        x: remote.center.x / 2.0,
        y: remote.center.y / 2.0,
    };
    assert_eq!(
        layout::hit_test(&placed, halved),
        None,
        "a logical-point cursor must not be expected to hit anything"
    );
}
