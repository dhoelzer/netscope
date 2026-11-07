//! Draws the whole scene with Vello CPU.

pub mod layout;
pub mod text;

use vello_cpu::color::{AlphaColor, Srgb};
use vello_cpu::kurbo::{Circle, Line, Point, Rect, Shape, Stroke};
use vello_cpu::{Pixmap, RenderContext, RenderSettings, Resources};

use crate::capture::parse::Transport;
use crate::model::{Model, RemoteHost};
use crate::net::{AddressKind, LocalNetwork, format_address};
use crate::render::layout::{LAN_LABEL_PITCH, Layout, describe_interface, lan_label};
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
const TEXT: Paint = rgb(0.90, 0.93, 0.98);
const DIM: Paint = rgb(0.55, 0.60, 0.68);
const WARN: Paint = rgb(0.98, 0.75, 0.30);

/// UI text sizes, scaled for readability at typical window sizes.
const TITLE_SIZE: f32 = 16.0;
const LABEL_SIZE: f32 = 14.5;
const DETAIL_SIZE: f32 = 12.5;
const HUD_SIZE: f32 = 14.5;

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

        self.draw_links(&placed, model, now);
        self.draw_host(&placed);
        self.draw_lan(&placed, network);
        self.draw_remotes(&placed, now);
        self.draw_hud(&placed, hud);

        self.ctx.flush();
        self.ctx.render(&mut self.pixmap, &mut self.resources);
        placed
    }

    // ---- links -------------------------------------------------------------

    fn draw_links(&mut self, placed: &Layout, model: &Model, now: std::time::Instant) {
        for flow in model.flows() {
            let opacity = Model::opacity(flow.last_seen, now);
            if opacity <= 0.001 {
                continue;
            }
            let Some(remote) = placed.remotes.iter().find(|r| r.host.addr == flow.remote) else {
                continue;
            };
            // The local end is the exact interface square that carried the
            // traffic; a flow reached through the LAN anchors on that circle.
            let anchor = match flow.local {
                Some(addr) => crate::render::layout::local_anchor(placed, addr, remote.center),
                None => crate::render::layout::lan_anchor(placed, remote.center),
            };
            let Some(anchor) = anchor else { continue };
            let start = crate::render::layout::rim(remote.center, remote.radius, anchor);

            self.ctx.set_stroke(Stroke {
                width: link_width(flow.packets),
                ..Stroke::default()
            });
            self.ctx.set_paint(with_alpha(REMOTE, opacity * 0.75));
            self.ctx.stroke_path(&line_path(start, anchor));
        }

        // Traffic between this host and the local network: one line between the
        // host panel and the local-network circle, weighted by all such flows.
        let (Some(host), Some(lan)) = (placed.host.as_ref(), placed.lan.as_ref()) else {
            return;
        };
        let mut packets = 0u64;
        let mut last_seen: Option<std::time::Instant> = None;
        for link in model.lan_links() {
            packets += link.packets;
            last_seen = Some(match last_seen {
                Some(t) => t.max(link.last_seen),
                None => link.last_seen,
            });
        }
        if let Some(last_seen) = last_seen {
            let opacity = Model::opacity(last_seen, now);
            if opacity > 0.001 {
                let from = crate::render::layout::host_anchor(placed, lan.center)
                    .unwrap_or(crate::render::layout::rect_center(host.rect));
                if let Some(to) = crate::render::layout::lan_anchor(placed, from) {
                    self.ctx.set_stroke(Stroke {
                        width: link_width(packets),
                        ..Stroke::default()
                    });
                    self.ctx.set_paint(with_alpha(LAN_RING, opacity * 0.85));
                    self.ctx.stroke_path(&line_path(from, to));
                }
            }
        }
    }

    // ---- host panel --------------------------------------------------------

    fn draw_host(&mut self, placed: &Layout) {
        let Some(host) = &placed.host else { return };
        let panel = host.rect;

        self.ctx.set_paint(HOST_BODY);
        self.ctx.fill_rect(&panel);
        self.ctx.set_stroke(Stroke::new(2.5));
        self.ctx.set_paint(HOST_RING);
        self.ctx.stroke_rect(&panel);

        self.ctx.set_paint(HOST_RING);
        self.font.draw(
            &mut self.ctx,
            &mut self.resources,
            Point {
                x: panel.x0 + 14.0,
                y: panel.y0 + 22.0,
            },
            "this host",
            TITLE_SIZE,
        );

        for (i, iface) in host.interfaces.iter().enumerate() {
            let tint = interface_tint(iface.kind);
            let square = iface.rect;

            self.ctx.set_paint(with_alpha(tint, 0.28));
            self.ctx.fill_rect(&square);
            self.ctx.set_stroke(Stroke::new(1.8));
            self.ctx.set_paint(tint);
            self.ctx.stroke_rect(&square);

            let label = self.font.fit(
                &describe_interface(&crate::net::LocalAddress {
                    iface: iface.iface.clone(),
                    ip: iface.addr,
                    prefix: None,
                    kind: iface.kind,
                }),
                LABEL_SIZE,
                host.label_budget as f32,
            );
            self.ctx.set_paint(TEXT);
            self.font.draw(
                &mut self.ctx,
                &mut self.resources,
                Point {
                    x: square.x1 + 10.0,
                    y: square.y1
                        - (square.height() - f64::from(self.font.line_height(LABEL_SIZE))) / 2.0
                        - f64::from(self.font.descent(LABEL_SIZE)),
                },
                &label,
                LABEL_SIZE,
            );
            let _ = i;
        }
    }

    // ---- local network -----------------------------------------------------

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

        let label = lan_label(network);
        self.ctx.set_paint(LAN_RING);
        self.font.draw_centered(
            &mut self.ctx,
            &mut self.resources,
            center,
            center.y - lan.radius - 8.0 - f64::from(self.font.descent(TITLE_SIZE)),
            &label,
            TITLE_SIZE,
        );

        for peer in &lan.hosts {
            let at = Point {
                x: center.x + peer.offset.x,
                y: center.y + peer.offset.y,
            };
            self.ctx.set_paint(with_alpha(LAN_RING, 0.30));
            self.ctx
                .fill_path(&Circle::new(at, peer.radius).to_path(0.1));
            self.ctx.set_stroke(Stroke::new(1.4));
            self.ctx.set_paint(LAN_RING);
            self.ctx
                .stroke_path(&Circle::new(at, peer.radius).to_path(0.1));

            if peer.labeled {
                let text = self.font.fit(
                    &format_address(peer.addr),
                    DETAIL_SIZE,
                    (LAN_LABEL_PITCH - 6.0) as f32,
                );
                self.ctx.set_paint(TEXT);
                self.font.draw_centered(
                    &mut self.ctx,
                    &mut self.resources,
                    at,
                    at.y + peer.radius + f64::from(self.font.ascent(DETAIL_SIZE)) - 2.0,
                    &text,
                    DETAIL_SIZE,
                );
            }
        }

        let hidden = lan.hidden;
        self.ctx.set_paint(DIM);
        self.font.draw_centered(
            &mut self.ctx,
            &mut self.resources,
            center,
            center.y + lan.radius - 10.0,
            &format!(
                "{} peer{}{}",
                lan.hosts.len(),
                if lan.hosts.len() == 1 { "" } else { "s" },
                if hidden > 0 {
                    format!(" · {hidden} not drawn")
                } else {
                    String::new()
                }
            ),
            DETAIL_SIZE,
        );
    }

    // ---- remote hosts ------------------------------------------------------

    fn draw_remotes(&mut self, placed: &Layout, now: std::time::Instant) {
        for remote in &placed.remotes {
            let opacity = Model::opacity(remote.host.last_seen, now);
            if opacity <= 0.001 {
                continue;
            }
            let center = remote.center;

            self.ctx.set_paint(with_alpha(REMOTE, 0.22 * opacity));
            self.ctx
                .fill_path(&Circle::new(center, remote.radius).to_path(0.1));
            self.ctx.set_stroke(Stroke::new(1.6));
            self.ctx.set_paint(with_alpha(REMOTE, opacity));
            self.ctx
                .stroke_path(&Circle::new(center, remote.radius).to_path(0.1));

            let label = format_address(remote.host.addr);
            let label = self
                .font
                .fit(&label, LABEL_SIZE, placed.label_budget as f32);
            self.ctx.set_paint(with_alpha(TEXT, opacity));
            self.font.draw(
                &mut self.ctx,
                &mut self.resources,
                Point {
                    x: center.x + remote.radius + 8.0,
                    y: center.y + f64::from(self.font.ascent(LABEL_SIZE)) / 2.0,
                },
                &label,
                LABEL_SIZE,
            );

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
                    x: center.x + remote.radius + 8.0,
                    y: center.y
                        + f64::from(self.font.ascent(LABEL_SIZE)) / 2.0
                        + f64::from(self.font.line_height(DETAIL_SIZE)),
                },
                &detail,
                DETAIL_SIZE,
            );
        }
    }

    // ---- HUD ---------------------------------------------------------------

    fn draw_hud(&mut self, placed: &Layout, hud: &Hud<'_>) {
        let size = HUD_SIZE;
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
        }
    }
}

fn link_width(packets: u64) -> f64 {
    1.4 + (packets.min(50) as f64 / 50.0) * 2.6
}

fn line_path(a: Point, b: Point) -> vello_cpu::kurbo::BezPath {
    Line::new(a, b).to_path(0.1)
}

fn with_alpha(color: Paint, alpha: f32) -> Paint {
    color.with_alpha(alpha)
}

fn interface_tint(kind: AddressKind) -> Paint {
    match kind {
        AddressKind::Loopback => rgb(0.62, 0.66, 0.74),
        AddressKind::LinkLocal => rgb(0.55, 0.80, 0.95),
        AddressKind::Global => rgb(0.45, 0.90, 0.70),
    }
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
