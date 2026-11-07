//! Draws the whole scene with Vello CPU.

pub mod layout;
pub mod text;

use vello_cpu::color::{AlphaColor, Srgb};
use vello_cpu::kurbo::{Cap, Circle, Line, Point, Rect, Shape, Stroke};
use vello_cpu::{Pixmap, RenderContext, RenderSettings, Resources};

use crate::capture::parse::Transport;
use crate::model::{Model, RemoteHost};
use crate::net::{AddressKind, LocalNetwork, format_address};
use crate::render::layout::{
    DETAIL_ROOM, DrawnLink, LAN_LABEL_PITCH, Layout, LinkKey, describe_interface, lan_label,
};
use crate::render::text::Font;
use crate::service::Service;
use std::collections::BTreeMap;
use std::net::IpAddr;

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
/// Hop guide lines: present but recessive, they are a ruler, not data.
const HOP_LINE: Paint = rgb(0.34, 0.40, 0.50);
const HOP_LABEL: Paint = rgb(0.62, 0.68, 0.78);

/// UI text sizes, scaled for readability at typical window sizes.
const TITLE_SIZE: f32 = 16.0;
const LABEL_SIZE: f32 = 14.5;
const DETAIL_SIZE: f32 = 12.5;
const HUD_SIZE: f32 = 14.5;
/// Services listed in the colour key before it reports how many are hidden.
const LEGEND_ROWS: usize = 8;
/// Pixels between parallel lines that share a pair of endpoints.
const LINK_FAN_GAP: f64 = 5.0;
/// Dash on/off lengths for the hop guide lines, in pixels.
const HOP_DASH_ON: f64 = 5.0;
const HOP_DASH_OFF: f64 = 5.0;
/// Floor for a remote address label; below this, truncation beats illegibility.
const MIN_LABEL_SIZE: f32 = 8.5;
/// Hop labels along the top, and the caption above them.
const HOP_LABEL_SIZE: f32 = 11.5;
/// Baseline for the hop labels, which sit at the very top of the display.
const HOP_LABEL_BASELINE: f64 = 26.0;
/// Baseline of the HUD title, which starts below the hop labels.
const HUD_BASELINE: f64 = 50.0;
/// Tooltip text: deliberately larger than every label on the scene, since it is
/// the thing the user asked to see by hovering.
const TOOLTIP_TITLE_SIZE: f32 = 19.0;
const TOOLTIP_SIZE: f32 = 16.5;
const TOOLTIP_BG: Paint = rgb(0.10, 0.13, 0.19);
const TOOLTIP_RING: Paint = rgb(0.55, 0.62, 0.74);

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
    /// log it or hit-test against it.
    ///
    /// `hover` is the pointer in window coordinates, or `None` when it is outside
    /// the window. Whatever it lands on gets a tooltip.
    pub fn draw(
        &mut self,
        network: &LocalNetwork,
        model: &Model,
        hud: &Hud<'_>,
        now: std::time::Instant,
        hover: Option<Point>,
    ) -> Layout {
        let (w, h) = (f64::from(self.width), f64::from(self.height));
        let remotes: Vec<RemoteHost> = model.remotes().cloned().collect();
        let lan: Vec<_> = model.lan().cloned().collect();
        let mut placed = crate::render::layout::build(w, h, network, &remotes, &lan, now);

        self.ctx.reset();
        self.ctx.set_paint(BG);
        self.ctx.fill_rect(&Rect::new(0.0, 0.0, w, h));

        self.draw_hop_lines(&placed, h);
        self.draw_links(&mut placed, model, now);
        self.draw_host(&placed);
        self.draw_lan(&placed, network);
        self.draw_remotes(&placed, now);
        self.draw_legend(model, now);
        self.draw_hud(&placed, hud);

        if let Some(at) = hover
            && let Some(target) = layout::hit_test(&placed, at)
        {
            self.draw_tooltip(&placed, model, network, &target, at, now);
        }

        self.ctx.flush();
        self.ctx.render(&mut self.pixmap, &mut self.resources);
        placed
    }

    // ---- links -------------------------------------------------------------

    /// One line per connection, coloured by the service it carries. Connections
    /// that share endpoints are fanned out sideways so each colour stays visible
    /// instead of being painted over by the next one.
    ///
    /// Each line is recorded in `placed.links` as it is drawn, which is what the
    /// pointer is matched against for link tooltips.
    fn draw_links(&mut self, placed: &mut Layout, model: &Model, now: std::time::Instant) {
        // BTreeMap so groups draw in a stable order; within a group the lines are
        // sorted by service. An unordered fan would reshuffle and flicker on
        // every frame.
        let mut groups: BTreeMap<(IpAddr, Option<IpAddr>), LinkGroup> = BTreeMap::new();

        for flow in model.flows() {
            let opacity = Model::link_opacity(flow.last_seen, now);
            if opacity <= 0.001 {
                continue;
            }
            let Some(remote) = placed.remotes.iter().find(|r| r.host.addr == flow.remote) else {
                continue;
            };
            // The local end is the exact interface square that carried the
            // traffic; a flow reached through the LAN anchors on that circle.
            let anchor = match flow.local {
                Some(addr) => layout::local_anchor(placed, addr, remote.center),
                None => layout::lan_anchor(placed, remote.center),
            };
            let Some(anchor) = anchor else { continue };
            let group = groups
                .entry((flow.remote, flow.local))
                .or_insert_with(|| LinkGroup {
                    start: layout::rim(remote.center, remote.radius, anchor),
                    anchor,
                    lines: Vec::new(),
                });
            group.lines.push((
                flow.service,
                flow.packets,
                opacity,
                LinkKey::Flow(flow.remote, flow.local, flow.service),
            ));
        }

        for group in groups.values_mut() {
            group.lines.sort_by_key(|line| line.0);
            self.stroke_fan(group, &mut placed.links);
        }

        // Traffic between this host and the local network: from the host panel to
        // the local-network circle, one line per connection.
        let Some(lan) = placed.lan.as_ref() else {
            return;
        };
        let Some(from) = layout::host_anchor(placed, lan.center) else {
            return;
        };
        let Some(to) = layout::lan_anchor(placed, from) else {
            return;
        };
        let mut lines: Vec<(Service, u64, f32, LinkKey)> = model
            .lan_links()
            .map(|link| {
                (
                    link.service,
                    link.packets,
                    Model::link_opacity(link.last_seen, now),
                    LinkKey::LanLink(link.peer, link.local, link.service),
                )
            })
            .filter(|(_, _, opacity, _)| *opacity > 0.001)
            .collect();
        lines.sort_by_key(|line| line.0);
        if !lines.is_empty() {
            self.stroke_fan(
                &LinkGroup {
                    start: from,
                    anchor: to,
                    lines,
                },
                &mut placed.links,
            );
        }
    }

    /// Strokes a group of connections sharing one pair of endpoints, fanning them
    /// sideways so every service colour stays visible.
    fn stroke_fan(&mut self, group: &LinkGroup, drawn: &mut Vec<DrawnLink>) {
        for (index, (service, packets, opacity, key)) in group.lines.iter().enumerate() {
            let (a, b) = fan(group.start, group.anchor, index, group.lines.len());
            let width = link_width(*packets);
            self.ctx.set_stroke(Stroke {
                width,
                ..Stroke::default()
            });
            self.ctx
                .set_paint(with_alpha(paint(service.rgb()), opacity * 0.85));
            self.ctx.stroke_path(&line_path(a, b));
            drawn.push(DrawnLink {
                a,
                b,
                service: *service,
                opacity: *opacity,
                width,
                key: *key,
            });
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

    /// The hop ruler: 20 evenly spaced dashed vertical lines with a hop count
    /// labelled at the top of each.
    ///
    /// The dashes come from kurbo's `Stroke::dash_pattern`, which `vello_cpu`
    /// honours when it flattens the stroke.
    fn draw_hop_lines(&mut self, placed: &Layout, height: f64) {
        let Some(first) = placed.hop_lines.first() else {
            return;
        };
        let top = HOP_LABEL_BASELINE;
        let line_top = top + 8.0;

        self.ctx.set_stroke(Stroke {
            dash_pattern: vec![HOP_DASH_ON, HOP_DASH_OFF].into(),
            start_cap: Cap::Butt,
            end_cap: Cap::Butt,
            ..Stroke::new(1.0)
        });
        self.ctx.set_paint(HOP_LINE);
        for line in &placed.hop_lines {
            self.ctx.stroke_path(&line_path(
                Point {
                    x: line.x,
                    y: line_top,
                },
                Point {
                    x: line.x,
                    y: height,
                },
            ));
        }

        // Labels centred on their line.
        self.ctx.set_paint(HOP_LABEL);
        for line in &placed.hop_lines {
            let text = line.hops.to_string();
            let w = f64::from(self.font.measure(&text, HOP_LABEL_SIZE));
            self.font.draw(
                &mut self.ctx,
                &mut self.resources,
                Point {
                    x: line.x - w / 2.0,
                    y: top,
                },
                &text,
                HOP_LABEL_SIZE,
            );
        }

        // Caption naming the axis, in the margin left of the furthest line.
        // The label row is otherwise full, so it stays short.
        self.ctx.set_paint(DIM);
        let caption = "hops";
        let w = f64::from(self.font.measure(caption, HOP_LABEL_SIZE));
        let furthest = placed.hop_lines.last().map(|l| l.x).unwrap_or(first.x);
        let caption_x = furthest - w - 12.0;
        if caption_x >= 8.0 {
            self.font.draw(
                &mut self.ctx,
                &mut self.resources,
                Point {
                    x: caption_x,
                    y: top,
                },
                caption,
                HOP_LABEL_SIZE,
            );
        }
    }

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

            // Labels go under the circle: with 20 guide lines sharing the
            // window there is no horizontal room beside it.
            let size = self.remote_label_size(remote.label_budget);
            let label = format_address(remote.host.addr);
            let label = self.font.fit(&label, size, remote.label_budget as f32);
            let w = f64::from(self.font.measure(&label, size));
            self.ctx.set_paint(with_alpha(TEXT, opacity));
            self.font.draw(
                &mut self.ctx,
                &mut self.resources,
                Point {
                    x: center.x - w / 2.0,
                    y: center.y + remote.radius + f64::from(self.font.ascent(size)) + 3.0,
                },
                &label,
                size,
            );

            if placed.detail_budget >= DETAIL_ROOM {
                let detail = format!(
                    "{} · {} · {}",
                    transport_tag(remote.host.last_transport),
                    remote.host.packets,
                    if remote.host.group {
                        "local group".to_string()
                    } else {
                        hop_tag(remote.host.hops)
                    },
                );
                let detail = self
                    .font
                    .fit(&detail, DETAIL_SIZE, remote.label_budget as f32);
                let w = f64::from(self.font.measure(&detail, DETAIL_SIZE));
                self.ctx.set_paint(with_alpha(DIM, opacity));
                self.font.draw(
                    &mut self.ctx,
                    &mut self.resources,
                    Point {
                        x: center.x - w / 2.0,
                        y: center.y
                            + remote.radius
                            + f64::from(self.font.ascent(size))
                            + 3.0
                            + f64::from(self.font.line_height(size)),
                    },
                    &detail,
                    DETAIL_SIZE,
                );
            }
        }
    }

    /// Font size for a remote address, shrunk when the room the label has is
    /// narrow enough that a full IPv4 address would collide with its neighbours.
    fn remote_label_size(&self, label_budget: f64) -> f32 {
        let widest = f64::from(self.font.measure("255.255.255.255", LABEL_SIZE));
        if widest <= label_budget {
            return LABEL_SIZE;
        }
        // Glyph width is proportional to size, so solve for the size that fits.
        (LABEL_SIZE * (label_budget / widest) as f32).max(MIN_LABEL_SIZE)
    }

    // ---- tooltip ---------------------------------------------------------

    /// Names whatever the pointer is on. Drawn last so it sits above every
    /// shape, including the one it describes.
    fn draw_tooltip(
        &mut self,
        placed: &Layout,
        model: &Model,
        network: &LocalNetwork,
        hit: &layout::Hit,
        at: Point,
        now: std::time::Instant,
    ) {
        let (title, lines) = match hit {
            layout::Hit::Remote(addr) => {
                let Some(host) = model.remote(*addr) else {
                    return;
                };
                remote_tooltip(host, now)
            }
            layout::Hit::LanPeer(addr) => {
                let Some(peer) = model.lan_peer(*addr) else {
                    return;
                };
                (
                    format_address(*addr),
                    vec![
                        "on the local network".to_string(),
                        format!("{} packets", format_count(peer.packets)),
                        format!(
                            "{} · {}",
                            format_bytes(peer.bytes),
                            age_tag(peer.last_seen, now)
                        ),
                    ],
                )
            }
            layout::Hit::Interface(addr) => {
                let Some(local) = network.addresses().iter().find(|a| a.ip == *addr) else {
                    return;
                };
                (
                    describe_interface(local),
                    vec![
                        format!("interface {}", local.iface),
                        format!(
                            "{} connection{} in use",
                            model.flows_on_interface(*addr),
                            if model.flows_on_interface(*addr) == 1 {
                                ""
                            } else {
                                "s"
                            }
                        ),
                    ],
                )
            }
            layout::Hit::HostPanel => (
                "this host".to_string(),
                vec![
                    format!(
                        "{} address{}",
                        network.addresses().len(),
                        if network.addresses().len() == 1 {
                            ""
                        } else {
                            "es"
                        }
                    ),
                    "the squares are its interfaces".to_string(),
                ],
            ),
            layout::Hit::LanCircle => (
                lan_label(network),
                vec![
                    format!(
                        "{} peer{} seen",
                        model.lan_count(),
                        if model.lan_count() == 1 { "" } else { "s" }
                    ),
                    "hosts on our networks, not our own addresses".to_string(),
                ],
            ),
            layout::Hit::Link(key) => match key {
                layout::LinkKey::Flow(remote, local, service) => {
                    let Some(flow) = model.flow(*remote, *local, *service) else {
                        return;
                    };
                    link_tooltip(
                        service,
                        &Directions {
                            to_packets: flow.to_remote_packets,
                            to_bytes: flow.to_remote_bytes,
                            from_packets: flow.from_remote_packets,
                            from_bytes: flow.from_remote_bytes,
                        },
                        &format_address(*remote),
                        local.map(format_address),
                        flow.last_seen,
                        now,
                    )
                }
                layout::LinkKey::LanLink(peer, local, service) => {
                    let Some(link) = model.lan_link(*peer, *local, *service) else {
                        return;
                    };
                    link_tooltip(
                        service,
                        &Directions {
                            to_packets: link.to_peer_packets,
                            to_bytes: link.to_peer_bytes,
                            from_packets: link.from_peer_packets,
                            from_bytes: link.from_peer_bytes,
                        },
                        &format_address(*peer),
                        Some(format_address(*local)),
                        link.last_seen,
                        now,
                    )
                }
            },
        };

        let Some(anchor) = layout::hit_anchor(placed, hit, at) else {
            return;
        };

        // Size the card to its widest line, then place it beside the shape,
        // flipping to the other side and clamping to the window so the card
        // never hides the thing being inspected or runs off screen.
        let pad_x = 14.0;
        let pad_y = 12.0;
        let gap = 14.0;
        let title_w = f64::from(self.font.measure(&title, TOOLTIP_TITLE_SIZE));
        let body_w = lines
            .iter()
            .map(|l| f64::from(self.font.measure(l, TOOLTIP_SIZE)))
            .fold(0.0, f64::max);
        let card_w = pad_x * 2.0 + title_w.max(body_w);
        let line_h = f64::from(self.font.line_height(TOOLTIP_SIZE));
        let card_h = pad_y * 2.0
            + f64::from(self.font.line_height(TOOLTIP_TITLE_SIZE))
            + line_h * lines.len() as f64;

        let (w, h) = (f64::from(self.width), f64::from(self.height));
        let mut x = anchor.x1 + gap;
        if x + card_w > w - 8.0 {
            x = anchor.x0 - gap - card_w;
        }
        x = x.clamp(8.0, (w - card_w - 8.0).max(8.0));
        let mut y = (anchor.y0 + anchor.y1) / 2.0 - card_h / 2.0;
        y = y.clamp(8.0, (h - card_h - 8.0).max(8.0));

        let card = Rect::new(x, y, x + card_w, y + card_h);
        self.ctx.set_paint(with_alpha(TOOLTIP_BG, 0.96));
        self.ctx.fill_path(&rounded_rect(card, 9.0));
        self.ctx.set_stroke(Stroke {
            width: 1.4,
            ..Stroke::default()
        });
        self.ctx.set_paint(TOOLTIP_RING);
        self.ctx.stroke_path(&rounded_rect(card, 9.0));

        let mut baseline = y + pad_y + f64::from(self.font.ascent(TOOLTIP_TITLE_SIZE));
        self.font.draw(
            &mut self.ctx,
            &mut self.resources,
            Point {
                x: x + pad_x,
                y: baseline,
            },
            &title,
            TOOLTIP_TITLE_SIZE,
        );
        baseline += f64::from(self.font.line_height(TOOLTIP_TITLE_SIZE));
        for line in &lines {
            self.ctx.set_paint(DIM);
            self.font.draw(
                &mut self.ctx,
                &mut self.resources,
                Point {
                    x: x + pad_x,
                    y: baseline,
                },
                line,
                TOOLTIP_SIZE,
            );
            baseline += line_h;
        }
    }

    // ---- service legend --------------------------------------------------

    /// Colour key for the services currently drawn, so line colours are readable.
    fn draw_legend(&mut self, model: &Model, now: std::time::Instant) {
        let mut services: Vec<(Service, f32)> = Vec::new();
        let mut note = |service: Service, opacity: f32| {
            if opacity <= 0.001 {
                return;
            }
            match services.iter_mut().find(|(s, _)| *s == service) {
                Some(entry) => entry.1 = entry.1.max(opacity),
                None => services.push((service, opacity)),
            }
        };
        for flow in model.flows() {
            note(flow.service, Model::link_opacity(flow.last_seen, now));
        }
        for link in model.lan_links() {
            note(link.service, Model::link_opacity(link.last_seen, now));
        }
        if services.is_empty() {
            return;
        }
        services.sort_by_key(|(service, _)| *service);

        let size = DETAIL_SIZE;
        let line_h = f64::from(self.font.line_height(size));
        let swatch = 11.0f64;
        let x = 16.0f64;
        let mut y = f64::from(self.height) - 16.0 - f64::from(self.font.descent(size));

        self.ctx.set_paint(DIM);
        self.font.draw(
            &mut self.ctx,
            &mut self.resources,
            Point { x, y: y - line_h },
            "traffic colour key",
            size,
        );

        for (service, opacity) in services.iter().rev().take(LEGEND_ROWS) {
            let colour = paint(service.rgb()).with_alpha(*opacity);
            self.ctx.set_paint(colour);
            self.ctx.fill_rect(&Rect::new(
                x,
                y - f64::from(self.font.ascent(size)) + 2.0,
                x + swatch,
                y - f64::from(self.font.ascent(size)) + 2.0 + swatch,
            ));
            self.ctx.set_paint(with_alpha(TEXT, *opacity));
            self.font.draw(
                &mut self.ctx,
                &mut self.resources,
                Point {
                    x: x + swatch + 8.0,
                    y,
                },
                &service.described(),
                size,
            );
            y -= line_h;
        }
        let extra = services.len().saturating_sub(LEGEND_ROWS);
        if extra > 0 {
            self.ctx.set_paint(DIM);
            self.font.draw(
                &mut self.ctx,
                &mut self.resources,
                Point { x, y },
                &format!("+ {extra} more"),
                size,
            );
        }
    }

    // ---- HUD ---------------------------------------------------------------

    fn draw_hud(&mut self, placed: &Layout, hud: &Hud<'_>) {
        let size = HUD_SIZE;
        let line_h = f64::from(self.font.line_height(size));
        let x = 16.0f64;
        let mut y = HUD_BASELINE;

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
        // The status line must not run under the hop labels along the top.
        let hud_width = placed
            .hop_lines
            .first()
            .map(|l| (l.x - 24.0).max(240.0))
            .unwrap_or(f64::from(self.width)) as f32;
        let quiet = if hud.quiet_for.is_finite() {
            format!("{:.1}s", hud.quiet_for)
        } else {
            "—".to_string()
        };
        self.font.draw(
            &mut self.ctx,
            &mut self.resources,
            Point { x, y },
            &self.font.fit(
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
                hud_width,
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

/// A pair of endpoints and the connections drawn between them.
struct LinkGroup {
    start: Point,
    anchor: Point,
    lines: Vec<(Service, u64, f32, LinkKey)>,
}

/// Offset for the `index`-th of `count` lines sharing one pair of endpoints,
/// shifting them sideways so parallel connections stay individually visible.
fn fan(a: Point, b: Point, index: usize, count: usize) -> (Point, Point) {
    if count < 2 {
        return (a, b);
    }
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1.0 {
        return (a, b);
    }
    // Unit normal to the segment.
    let (nx, ny) = (-dy / len, dx / len);
    let step = (index as f64) - (count as f64 - 1.0) / 2.0;
    let offset = step * LINK_FAN_GAP;
    (
        Point {
            x: a.x + nx * offset,
            y: a.y + ny * offset,
        },
        Point {
            x: b.x + nx * offset,
            y: b.y + ny * offset,
        },
    )
}

fn line_path(a: Point, b: Point) -> vello_cpu::kurbo::BezPath {
    Line::new(a, b).to_path(0.1)
}

fn with_alpha(color: Paint, alpha: f32) -> Paint {
    color.with_alpha(alpha)
}

/// Maps a colour-module `[f32; 3]` into Vello's paint type.
const fn paint(channels: [f32; 3]) -> Paint {
    rgb(channels[0], channels[1], channels[2])
}

/// Squares stay deliberately muted: they are chrome, and a saturated fill here
/// would be mistaken for one of the protocol-coloured traffic lines.
fn interface_tint(kind: AddressKind) -> Paint {
    match kind {
        AddressKind::Loopback => rgb(0.30, 0.33, 0.40),
        AddressKind::LinkLocal => rgb(0.28, 0.40, 0.50),
        AddressKind::Global => rgb(0.26, 0.44, 0.42),
    }
}

/// Distance readout for a remote host, before any inbound packet has revealed
/// its TTL.
fn hop_tag(hops: Option<u32>) -> String {
    match hops {
        Some(h) if h >= 32 => "32+ hops".to_string(),
        Some(h) => format!("{h} hop{}", if h == 1 { "" } else { "s" }),
        None => "hops ?".to_string(),
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

// ---- tooltip text --------------------------------------------------------

/// Title and body lines for a remote circle.
fn remote_tooltip(host: &RemoteHost, now: std::time::Instant) -> (String, Vec<String>) {
    let distance = match (host.group, host.hops) {
        (true, _) => "broadcast / multicast group".to_string(),
        (false, Some(hops)) => format!("{hops} hop{}", if hops == 1 { "" } else { "s" }),
        (false, None) => "distance unknown".to_string(),
    };
    (
        format_address(host.addr),
        vec![
            distance,
            format!(
                "{} packets · {}",
                format_count(host.packets),
                format_bytes(host.bytes)
            ),
            format!(
                "{} · {} via LAN",
                Service::new(host.last_transport, None).label(),
                format_count(host.lan_packets)
            ),
            format!("last packet {}", age_tag(host.last_seen, now)),
        ],
    )
}

/// Title and body lines for a traffic line, with the two directions counted
/// separately so a one-sided connection is obvious.
/// Traffic on one connection, counted per direction.
struct Directions {
    to_packets: u64,
    to_bytes: u64,
    from_packets: u64,
    from_bytes: u64,
}

fn link_tooltip(
    service: &Service,
    counts: &Directions,
    far: &str,
    near: Option<String>,
    last_seen: std::time::Instant,
    now: std::time::Instant,
) -> (String, Vec<String>) {
    let near = near.unwrap_or_else(|| "local network".to_string());
    (
        service.described(),
        vec![
            format!("{near} <-> {far}"),
            format!(
                "out {} pkts / {} - in {} pkts / {}",
                format_count(counts.to_packets),
                format_bytes(counts.to_bytes),
                format_count(counts.from_packets),
                format_bytes(counts.from_bytes)
            ),
            format!("last packet {}", age_tag(last_seen, now)),
        ],
    )
}

/// "3.4s ago" style age, coarse enough to read at a glance.
fn age_tag(then: std::time::Instant, now: std::time::Instant) -> String {
    let secs = now.duration_since(then).as_secs_f64();
    if secs < 1.0 {
        "just now".to_string()
    } else if secs < 60.0 {
        format!("{secs:.0}s ago")
    } else {
        format!("{:.0}min ago", secs / 60.0)
    }
}

/// Bytes with a binary suffix, so a busy link does not print a 9-digit number.
fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Thousands-separated count, which keeps a busy host's packet total readable.
fn format_count(count: u64) -> String {
    let digits = count.to_string();
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push('_');
        }
        out.push(ch);
    }
    out
}

/// A rectangle with rounded corners, as a path.
fn rounded_rect(rect: Rect, radius: f64) -> vello_cpu::kurbo::BezPath {
    vello_cpu::kurbo::RoundedRect::new(rect.x0, rect.y0, rect.x1, rect.y1, radius).to_path(0.1)
}
