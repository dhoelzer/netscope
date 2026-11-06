//! Draws the whole scene with Vello CPU.

pub mod layout;
pub mod text;

use vello_cpu::color::{AlphaColor, Srgb};
use vello_cpu::kurbo::{Circle, Line, Point, Rect, Shape, Stroke};
use vello_cpu::{Pixmap, RenderContext, RenderSettings, Resources};

use crate::capture::parse::Transport;
use crate::model::{Model, RemoteHost};
use crate::net::{LocalNetwork, format_address};
use crate::render::layout::{Layout, REMOTE_RADIUS, describe_interface, lan_label};
use crate::render::text::Font;

type Paint = AlphaColor<Srgb>;

const fn rgb(r: f32, g: f32, b: f32) -> Paint {
    Paint::new([r, g, b, 1.0])
}

const BG: Paint = rgb(0.047, 0.063, 0.094);
const HOST_RING: Paint = rgb(0.85, 0.88, 0.95);
const HOST_BODY: Paint = rgb(0.16, 0.20, 0.28);
const LAN_BODY: Paint = rgb(0.10, 0.22, 0.30);
const LAN_RING: Paint = rgb(0.35, 0.72, 0.95);
const REMOTE: Paint = rgb(0.98, 0.62, 0.25);
const REMOTE_LAN: Paint = rgb(0.55, 0.80, 0.95);
const TEXT: Paint = rgb(0.90, 0.93, 0.98);
const DIM: Paint = rgb(0.55, 0.60, 0.68);
const WARN: Paint = rgb(0.98, 0.75, 0.30);

/// Status line shown in the HUD.
pub struct Hud<'a> {
    pub source: &'a str,
    pub frames_seen: u64,
    pub frames_decoded: u64,
    pub remotes: usize,
    pub lan_peers: usize,
    pub quiet_for: f64,
    pub fps: f64,
    pub notice: Option<&'a str>,
}

/// Owns the reusable Vello CPU objects for one window.
pub struct Scene {
    ctx: RenderContext,
    resources: Resources,
    pixmap: Pixmap,
    font: Font,
    width: u16,
    height: u16,
}

impl Scene {
    #[must_use]
    pub fn new(width: u16, height: u16) -> Self {
        Self {
            ctx: RenderContext::new_with(
                width,
                height,
                RenderSettings {
                    num_threads: 0, // 0 = all cores, with the multithreading feature on
                    ..Default::default()
                },
            ),
            resources: Resources::new(),
            pixmap: Pixmap::new(width, height),
            font: Font::bundled(),
            width,
            height,
        }
    }

    pub fn resize(&mut self, width: u16, height: u16) {
        self.width = width;
        self.height = height;
        self.pixmap.resize(width, height);
        self.ctx.reset_and_resize(width, height);
    }

    #[must_use]
    pub fn pixmap(&self) -> &Pixmap {
        &self.pixmap
    }

    /// Renders one frame and returns the layout that was drawn, so callers can
    /// hit-test or log it.
    pub fn draw(
        &mut self,
        network: &LocalNetwork,
        model: &Model,
        hud: &Hud<'_>,
        now: std::time::Instant,
    ) -> Layout {
        let (w, h) = (f64::from(self.width), f64::from(self.height));
        let remotes: Vec<RemoteHost> = model.remotes().cloned().collect();
        let lan: Vec<_> = model.lan().cloned().collect();
        let placed = crate::render::layout::build(w, h, network, &remotes, &lan);

        self.ctx.reset();
        self.ctx.set_paint(BG);
        self.ctx.fill_rect(&Rect::new(0.0, 0.0, w, h));

        self.draw_links(&placed, model, network, now);
        self.draw_host(&placed, network);
        self.draw_lan(&placed, network);
        self.draw_remotes(&placed, now);
        self.draw_hud(&placed, hud);

        self.ctx.flush();
        self.ctx.render(&mut self.pixmap, &mut self.resources);
        placed
    }

    // ---- links -------------------------------------------------------------

    fn draw_links(
        &mut self,
        placed: &Layout,
        model: &Model,
        _network: &LocalNetwork,
        now: std::time::Instant,
    ) {
        let lan_center = placed.lan.as_ref().map(|l| l.center);
        for flow in model.flows() {
            let opacity = Model::opacity(flow.last_seen, now);
            if opacity <= 0.001 {
                continue;
            }
            let Some(remote) = placed.remotes.iter().find(|r| r.host.addr == flow.remote) else {
                continue;
            };
            let anchor = match flow.local {
                Some(addr) => crate::render::layout::local_anchor(placed, addr, remote.center),
                None => lan_center.map(|c| {
                    crate::render::layout::rim(
                        c,
                        placed.lan.as_ref().unwrap().radius,
                        remote.center,
                    )
                }),
            };
            let Some(anchor) = anchor else { continue };
            let start = crate::render::layout::rim(remote.center, remote.radius, anchor);

            let width = 1.0 + (flow.packets.min(50) as f64 / 50.0) * 2.5;
            self.ctx.set_stroke(Stroke {
                width,
                ..Stroke::default()
            });
            self.ctx.set_paint(with_alpha(REMOTE, opacity * 0.75));
            self.ctx.stroke_path(&line_path(start, anchor));
        }

        // LAN peer <-> one of our interfaces: a link between the interface ring
        // and the local-network circle, on the right of the host.
        let Some(lan) = placed.lan.as_ref() else {
            return;
        };
        for link in model.lan_links() {
            let opacity = Model::opacity(link.last_seen, now);
            if opacity <= 0.001 {
                continue;
            }
            let Some(host) = placed.host.as_ref() else {
                continue;
            };
            let Some(to_interface) =
                crate::render::layout::local_anchor(placed, link.local, lan.center)
            else {
                continue;
            };
            let from_lan = crate::render::layout::rim(lan.center, lan.radius, host.center);

            self.ctx.set_stroke(Stroke {
                width: 1.0 + (link.packets.min(50) as f64 / 50.0) * 2.0,
                ..Stroke::default()
            });
            self.ctx.set_paint(with_alpha(LAN_RING, opacity * 0.8));
            self.ctx.stroke_path(&line_path(from_lan, to_interface));
        }
    }

    // ---- host + interfaces -------------------------------------------------

    fn draw_host(&mut self, placed: &Layout, network: &LocalNetwork) {
        let Some(host) = &placed.host else { return };
        let center = host.center;

        // Filled body so the nested rings read as one object.
        self.ctx.set_paint(HOST_BODY);
        self.ctx
            .fill_path(&Circle::new(center, host.radius).to_path(0.1));

        self.ctx.set_stroke(Stroke::new(2.5));
        self.ctx.set_paint(HOST_RING);
        self.ctx
            .stroke_path(&Circle::new(center, host.radius).to_path(0.1));

        // One ring per interface address, smallest = last listed.
        for (i, iface) in host.interfaces.iter().enumerate() {
            let tint = interface_tint(i);
            self.ctx.set_paint(with_alpha(tint, 0.18));
            self.ctx
                .fill_path(&Circle::new(center, iface.radius).to_path(0.1));
            self.ctx.set_stroke(Stroke::new(1.8));
            self.ctx.set_paint(tint);
            self.ctx
                .stroke_path(&Circle::new(center, iface.radius).to_path(0.1));
        }

        // Labels: host name above, then one line per interface address.
        let size = 13.0f32;
        let line_h = f64::from(self.font.line_height(size));
        let title = "this host";
        self.ctx.set_paint(HOST_RING);
        self.font.draw_centered(
            &mut self.ctx,
            &mut self.resources,
            center,
            center.y - host.radius - 12.0 - f64::from(self.font.descent(size)),
            title,
            size,
        );

        let mut y = center.y + host.radius + 18.0 + f64::from(self.font.ascent(size));
        for iface in &host.interfaces {
            let label = self.font.fit(
                &describe_interface(&crate::net::LocalAddress {
                    iface: iface.iface.clone(),
                    ip: iface.addr,
                    prefix: None,
                    kind: crate::net::AddressKind::Global,
                }),
                size,
                320.0,
            );
            let tint = interface_tint(
                host.interfaces
                    .iter()
                    .position(|i| i.addr == iface.addr)
                    .unwrap_or(0),
            );
            self.ctx.set_paint(tint);
            self.font.draw(
                &mut self.ctx,
                &mut self.resources,
                Point {
                    x: center.x - 160.0,
                    y,
                },
                &label,
                size,
            );
            y += line_h;
        }

        let loopbacks = network.loopback_addresses().len();
        if loopbacks > 0 {
            self.ctx.set_paint(DIM);
            self.font.draw_centered(
                &mut self.ctx,
                &mut self.resources,
                center,
                center.y + 4.0,
                &format!("{loopbacks} loopback"),
                11.0,
            );
        }
    }

    fn draw_lan(&mut self, placed: &Layout, network: &LocalNetwork) {
        let Some(lan) = &placed.lan else { return };
        let center = lan.center;

        self.ctx.set_paint(LAN_BODY);
        self.ctx
            .fill_path(&Circle::new(center, lan.radius).to_path(0.1));
        self.ctx.set_stroke(Stroke::new(2.0));
        self.ctx.set_paint(LAN_RING);
        self.ctx
            .stroke_path(&Circle::new(center, lan.radius).to_path(0.1));

        let size = 13.0f32;
        self.ctx.set_paint(LAN_RING);
        self.font.draw_centered(
            &mut self.ctx,
            &mut self.resources,
            center,
            center.y - lan.radius - 12.0 - f64::from(self.font.descent(size)),
            "local network",
            size,
        );

        let label = lan_label(network);
        self.ctx.set_paint(TEXT);
        self.font.draw_centered(
            &mut self.ctx,
            &mut self.resources,
            center,
            center.y + f64::from(self.font.ascent(size)) / 2.0,
            &label,
            size,
        );

        let count = lan.hosts.len();
        self.ctx.set_paint(DIM);
        self.font.draw_centered(
            &mut self.ctx,
            &mut self.resources,
            center,
            center.y
                + f64::from(self.font.ascent(size)) / 2.0
                + f64::from(self.font.line_height(11.0)),
            &format!("{count} peer{}", if count == 1 { "" } else { "s" }),
            11.0,
        );
    }

    fn draw_remotes(&mut self, placed: &Layout, now: std::time::Instant) {
        let size = 12.0f32;
        for remote in &placed.remotes {
            let opacity = Model::opacity(remote.host.last_seen, now);
            if opacity <= 0.001 {
                continue;
            }
            let center = remote.center;
            // A host that also talks to LAN peers gets a cooler tint so the two
            // kinds of link are distinguishable.
            let tint = if remote.host.lan_packets > 0 {
                REMOTE_LAN
            } else {
                REMOTE
            };

            self.ctx.set_paint(with_alpha(tint, 0.22 * opacity));
            self.ctx
                .fill_path(&Circle::new(center, remote.radius).to_path(0.1));
            self.ctx.set_stroke(Stroke::new(1.6));
            self.ctx.set_paint(with_alpha(tint, opacity));
            self.ctx
                .stroke_path(&Circle::new(center, remote.radius).to_path(0.1));

            let label = format_address(remote.host.addr);
            let label = self.font.fit(&label, size, placed.label_budget as f32);
            self.ctx.set_paint(with_alpha(TEXT, opacity));
            let width = f64::from(self.font.measure(&label, size));
            self.font.draw(
                &mut self.ctx,
                &mut self.resources,
                Point {
                    x: center.x + REMOTE_RADIUS + 8.0,
                    y: center.y + f64::from(self.font.ascent(size)) / 2.0,
                },
                &label,
                size,
            );
            let _ = width;

            let detail = format!(
                "{} · {}",
                transport_tag(remote.host.last_transport),
                remote.host.packets
            );
            self.ctx.set_paint(with_alpha(DIM, opacity));
            self.font.draw(
                &mut self.ctx,
                &mut self.resources,
                Point {
                    x: center.x + REMOTE_RADIUS + 8.0,
                    y: center.y
                        + f64::from(self.font.ascent(size)) / 2.0
                        + f64::from(self.font.line_height(10.0)),
                },
                &detail,
                10.0,
            );
        }
    }

    fn draw_hud(&mut self, placed: &Layout, hud: &Hud<'_>) {
        let size = 12.0f32;
        let line_h = f64::from(self.font.line_height(size));
        let x = 16.0f64;
        let mut y = 16.0 + f64::from(self.font.ascent(size));

        self.ctx.set_paint(TEXT);
        self.font.draw(
            &mut self.ctx,
            &mut self.resources,
            Point { x, y },
            &format!("netscope — {}", hud.source),
            size,
        );
        y += line_h;

        self.ctx.set_paint(DIM);
        let quiet = if hud.quiet_for.is_finite() {
            format!("{:.1}s", hud.quiet_for)
        } else {
            "—".to_string()
        };
        self.font.draw(
            &mut self.ctx,
            &mut self.resources,
            Point { x, y },
            &format!(
                "frames {} · decoded {} · remote {} · lan {} · last packet {} · {:.0} fps{}",
                hud.frames_seen,
                hud.frames_decoded,
                hud.remotes,
                hud.lan_peers,
                quiet,
                hud.fps,
                if placed.hidden > 0 {
                    format!(" · {} not drawn", placed.hidden)
                } else {
                    String::new()
                }
            ),
            size,
        );
        y += line_h;

        if let Some(notice) = hud.notice {
            self.ctx.set_paint(WARN);
            self.font.draw(
                &mut self.ctx,
                &mut self.resources,
                Point { x, y },
                notice,
                size,
            );
            y += line_h;
        }

        self.ctx.set_paint(DIM);
        self.font.draw(
            &mut self.ctx,
            &mut self.resources,
            Point { x, y },
            "q / esc quit · space re-discover interfaces",
            size,
        );
        let _ = placed;
    }
}

fn with_alpha(color: Paint, alpha: f32) -> Paint {
    color.with_alpha(alpha)
}

fn interface_tint(index: usize) -> Paint {
    const PALETTE: [Paint; 4] = [
        rgb(0.45, 0.85, 0.55),
        rgb(0.95, 0.80, 0.35),
        rgb(0.70, 0.55, 0.95),
        rgb(0.95, 0.50, 0.60),
    ];
    PALETTE[index % PALETTE.len()]
}

fn transport_tag(transport: Transport) -> &'static str {
    match transport {
        Transport::Tcp => "tcp",
        Transport::Udp => "udp",
        Transport::Icmp => "icmp",
        Transport::IcmpV6 => "icmp6",
        Transport::Other(_) => "ip",
        Transport::None => "ip",
    }
}

fn line_path(a: Point, b: Point) -> vello_cpu::kurbo::BezPath {
    Line::new(a, b).to_path(0.1)
}
