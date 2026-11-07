// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of netscope; see LICENSE.
//! Where things go on screen.
//!
//! The host is a vertical rectangle in the upper right holding one square per
//! local address; the local network is a large circle in the lower right
//! holding one circle per LAN peer; remote hosts fill columns down the left.
//! Links join a remote circle to the exact interface square it talked to, or
//! the host rectangle to the local-network circle.

use std::net::IpAddr;

use vello_cpu::kurbo::{Point, Rect};

use crate::model::{LanHost, RemoteHost};
use crate::net::{AddressKind, LocalAddress, LocalNetwork};
use crate::service::Service;

pub const HOST_TOP_MARGIN: f64 = 56.0;
pub const HOST_RIGHT_MARGIN: f64 = 28.0;
/// Room kept beside each square for its address label.
pub const HOST_LABEL_WIDTH: f64 = 132.0;
/// Side of an interface square when there is room for it.
pub const MAX_SQUARE: f64 = 56.0;
pub const MIN_SQUARE: f64 = 20.0;
/// Below this square size, remaining addresses are dropped rather than drawn.
pub const MIN_LABELED_SQUARE: f64 = 24.0;

/// Radius bounds for the local-network circle.
pub const LAN_MIN_RADIUS: f64 = 96.0;
pub const LAN_MAX_RADIUS: f64 = 190.0;
/// Gap the local-network circle must leave between itself and the 1-hop guide
/// line: that gap is the lane where broadcast and multicast groups are drawn,
/// so the circle shrinks to keep it usable rather than swallow it.
pub const LAN_LANE_RESERVE: f64 = 120.0;
pub const LAN_MARGIN: f64 = 28.0;
/// Vertical room kept for the LAN caption between panel and circle.
pub const LAN_TITLE_RESERVE: f64 = 34.0;
/// Cell pitch bounds for the circles packed inside the LAN circle.
pub const MAX_LAN_PITCH: f64 = 68.0;
pub const MIN_LAN_PITCH: f64 = 26.0;
/// Pitch needed before per-peer address labels are drawn.
pub const LAN_LABEL_PITCH: f64 = 50.0;

/// Radius of a remote-host circle at full spacing.
pub const REMOTE_RADIUS: f64 = 26.0;
/// Vertical spacing between remote-host slots when there is room for it.
pub const REMOTE_SLOT: f64 = 62.0;
/// Smallest slot still worth drawing; past this, hosts are dropped instead.
pub const MIN_REMOTE_SLOT: f64 = 34.0;
/// Horizontal spacing between remote columns.
pub const REMOTE_COLUMN_PITCH: f64 = 200.0;
/// Left margin for the leftmost hop guide line.
pub const HOP_LEFT_MARGIN: f64 = 76.0;
/// Left inset of the bucket for hosts whose distance is unknown or beyond the
/// last guide line.
pub const FAR_LEFT_MARGIN: f64 = 22.0;
/// Number of vertical hop guide lines.
pub const HOP_LINES: usize = 20;
/// Room kept between a near-lane label and the shapes that bound the lane.
pub const NEAR_LANE_LABEL_PAD: f64 = 12.0;
/// The rightmost guide line sits this fraction of the window width in from the
/// right edge; the rest are spaced evenly leftward from it.
pub const HOP_RIGHT_FRACTION: f64 = 0.25;
/// Seconds a circle takes to slide from the left edge onto its guide line.
pub const HOP_SLIDE_SECONDS: f64 = 1.2;
/// Vertical room a detail line needs under a circle.
pub const DETAIL_ROOM: f64 = 30.0;
/// Bottom-left corner kept clear of remote circles for the traffic colour key.
pub const LEGEND_RESERVE: f64 = 150.0;
/// Guide lines this far from the left edge have to stop above the colour key.
pub const LEGEND_WIDTH: f64 = 210.0;
/// Vertical room kept at the top of the display for the HUD text.
pub const HUD_RESERVE: f64 = 96.0;
/// How far from a traffic line the pointer still counts as being on it. Lines
/// are thin and diagonal, so hit-testing them needs a band rather than a box.
pub const LINK_HIT_TOLERANCE: f64 = 6.0;

#[derive(Debug, Clone)]
pub struct PlacedInterface {
    pub addr: IpAddr,
    pub iface: String,
    pub kind: AddressKind,
    /// The square standing for this address, inside the host panel.
    pub rect: Rect,
}

#[derive(Debug, Clone)]
pub struct PlacedHost {
    /// The vertical rectangle standing for the host itself.
    pub rect: Rect,
    pub interfaces: Vec<PlacedInterface>,
    /// Horizontal room left for an address label beside its square.
    pub label_budget: f64,
}

#[derive(Debug, Clone)]
pub struct PlacedLanHost {
    pub addr: IpAddr,
    /// Position relative to the LAN circle's centre.
    pub offset: Point,
    pub radius: f64,
    /// Whether there is room to label this peer.
    pub labeled: bool,
}

#[derive(Debug, Clone)]
pub struct PlacedLan {
    pub center: Point,
    pub radius: f64,
    pub hosts: Vec<PlacedLanHost>,
    /// LAN peers that did not fit inside the circle and were not drawn.
    pub hidden: usize,
}

/// Which band of the display a remote circle was put in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteBand {
    /// A hop guide line; index 0 is the 1-hop line.
    Hop(usize),
    /// Broadcast or multicast group, parked beside the local network.
    Near,
    /// Distance unknown, or further than the last guide line.
    Far,
}

#[derive(Debug, Clone)]
pub struct PlacedRemote {
    pub center: Point,
    pub radius: f64,
    pub host: RemoteHost,
    /// Guide line this host sits on, `None` for a band with no line.
    pub slot: Option<usize>,
    /// Which band the circle belongs to.
    pub band: RemoteBand,
    /// Horizontal room this circle's label may use.
    pub label_budget: f64,
}

/// One vertical hop guide line and the hop count it stands for.
#[derive(Debug, Clone)]
pub struct HopLine {
    pub x: f64,
    pub hops: usize,
}

/// X positions of the guide lines, index 0 being the 1-hop line (rightmost).
#[must_use]
pub fn hop_line_positions(width: f64) -> Vec<f64> {
    let right = width * (1.0 - HOP_RIGHT_FRACTION);
    let left = HOP_LEFT_MARGIN.min(right);
    let pitch = (right - left) / (HOP_LINES - 1) as f64;
    (0..HOP_LINES).map(|i| right - i as f64 * pitch).collect()
}

/// X of the lane for group traffic: the midpoint of the gap between the 1-hop
/// guide line and the local-network circle's rim. If a window is too narrow to
/// leave a real gap, the lane stays just right of the line, which is still the
/// local side of the ruler.
#[must_use]
pub fn near_lane_x(line_one: f64, lan_rim: f64) -> f64 {
    if lan_rim > line_one {
        (line_one + lan_rim) / 2.0
    } else {
        line_one + 12.0
    }
}

/// The guide line index for a hop count, or `None` when the host belongs at the
/// left edge: distance unknown, or further than the last line.
#[must_use]
pub fn hop_slot(hops: Option<u32>) -> Option<usize> {
    match hops {
        Some(h) if (1..=HOP_LINES as u32).contains(&h) => Some((h - 1) as usize),
        _ => None,
    }
}

#[derive(Debug, Clone, Default)]
pub struct Layout {
    pub host: Option<PlacedHost>,
    pub lan: Option<PlacedLan>,
    pub remotes: Vec<PlacedRemote>,
    /// Remote hosts that did not fit in any column and were not drawn.
    pub hidden: usize,
    /// Vertical pitch actually used between remote slots.
    pub slot: f64,
    /// Horizontal room left for a remote label before it hits the next column.
    pub label_budget: f64,
    /// Bounding box actually used, for the "fit" readout in the HUD.
    pub used: Rect,
    /// The hop guide lines, right to left.
    pub hop_lines: Vec<HopLine>,
    /// X of the left-edge bucket.
    pub far_x: f64,
    /// X of the lane holding broadcast and multicast groups.
    pub near_x: f64,
    /// Vertical room under a circle for its detail line, 0 when there is none.
    pub detail_budget: f64,
    /// Traffic lines actually drawn, after fanning. Rendering and hit-testing
    /// read the same geometry, so a tooltip can only describe a line that is
    /// really on screen.
    pub links: Vec<DrawnLink>,
}

/// One drawn traffic line, after fanning, plus the connection it stands for.
#[derive(Debug, Clone)]
pub struct DrawnLink {
    pub a: Point,
    pub b: Point,
    pub service: Service,
    pub opacity: f32,
    pub width: f64,
    pub key: LinkKey,
}

/// Names the connection a drawn line represents, so a hit on a line can be
/// looked back up in the model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LinkKey {
    Flow(IpAddr, Option<IpAddr>, Service),
    LanLink(IpAddr, IpAddr, Service),
}

/// What the pointer is sitting on.
#[derive(Debug, Clone, PartialEq)]
pub enum Hit {
    Remote(IpAddr),
    LanPeer(IpAddr),
    Interface(IpAddr),
    HostPanel,
    LanCircle,
    Link(LinkKey),
}
/// Places every shape for the current frame.
///
/// `now` drives the slide-in animation that carries a circle from the left edge
/// onto its guide line once the distance becomes known.
#[must_use]
pub fn build(
    width: f64,
    height: f64,
    network: &LocalNetwork,
    remotes: &[RemoteHost],
    lan: &[LanHost],
    now: std::time::Instant,
) -> Layout {
    let mut layout = Layout::default();
    let vertical_pad = 40.0;

    let positions = hop_line_positions(width);
    let host = place_host(width, height, network);
    // The circle is placed knowing where the 1-hop line is, so it can shrink to
    // leave the group lane open rather than cover it.
    let lan_circle = place_lan(width, height, host.rect.y1, lan, positions[0]);
    let lan_rim = lan_circle.center.x - lan_circle.radius;
    layout.host = Some(host);
    layout.lan = Some(lan_circle);

    layout.far_x = FAR_LEFT_MARGIN;
    // Group traffic is parked in the region the 1-hop line and the
    // local-network circle leave between them.
    layout.near_x = near_lane_x(positions[0], lan_rim);
    layout.hop_lines = positions
        .iter()
        .enumerate()
        .map(|(i, x)| HopLine { x: *x, hops: i + 1 })
        .collect();

    // Bucket hosts by guide line, freshest first so the rows that matter sit on
    // top. Hosts with no usable TTL reading share the left-edge bucket.
    let mut by_slot: Vec<Vec<&RemoteHost>> = vec![Vec::new(); HOP_LINES];
    let mut far: Vec<&RemoteHost> = Vec::new();
    let mut near: Vec<&RemoteHost> = Vec::new();
    for host in remotes {
        // Groups bypass the hop ruler entirely: their TTL is not a distance.
        if host.group {
            near.push(host);
        } else {
            match hop_slot(host.hops) {
                Some(i) => by_slot[i].push(host),
                None => far.push(host),
            }
        }
    }
    sort_fresh(&mut far);
    sort_fresh(&mut near);
    for bucket in &mut by_slot {
        sort_fresh(bucket);
    }

    // One vertical pitch everywhere so rows line up across the display. It
    // shrinks to fit the tallest bucket before any host is dropped.
    let grid_top = HUD_RESERVE + 8.0;
    let grid_bottom = height - vertical_pad;
    let tallest = by_slot
        .iter()
        .map(Vec::len)
        .chain(std::iter::once(far.len()))
        .chain(std::iter::once(near.len()))
        .max()
        .unwrap_or(0);
    let band = (grid_bottom - grid_top).max(MIN_REMOTE_SLOT);

    // Shrink the row pitch until the tallest bucket fits the full-height band;
    // bands narrowed by the right-hand shapes drop rows on their own.
    let mut slot = REMOTE_SLOT;
    if tallest > (band / slot).floor() as usize {
        slot = (band / tallest as f64).clamp(MIN_REMOTE_SLOT, REMOTE_SLOT);
    }
    // The guide-line pitch is tighter than the row pitch once 20 lines share
    // the window, so it bounds the circle radius; otherwise neighbouring lines'
    // circles would touch.
    let line_pitch = positions
        .get(1)
        .map(|p| positions[0] - p)
        .unwrap_or(REMOTE_COLUMN_PITCH);
    let radius = (slot.min(line_pitch) * 0.34).clamp(8.0, REMOTE_RADIUS);

    // Each line gets its own vertical band: a line that passes behind the host
    // panel or the local-network circle stops short of it, so circles on that
    // line never sit on top of those shapes.
    let lan = layout.lan.clone().unwrap();
    let panel = layout.host.clone().unwrap();
    // `below_panel` is only set for the near lane: a circle there has to clear
    // the host panel from underneath, because the lane runs under it. Every
    // other line stops above the panel instead.
    let band_for = |x: f64, below_panel: bool| -> (f64, f64) {
        let mut top = grid_top + radius;
        let mut bottom = grid_bottom;
        let dx = x - lan.center.x;
        if dx.abs() < lan.radius {
            let half = (lan.radius * lan.radius - dx * dx).sqrt();
            bottom = bottom.min(lan.center.y - half - radius - 6.0);
        }
        if x > panel.rect.x0 - radius {
            if below_panel {
                top = top.max(panel.rect.y1 + radius + 6.0);
            } else {
                bottom = bottom.min(panel.rect.y1 - radius - 6.0);
            }
        }
        // The colour key owns the bottom-left corner.
        if x < LEGEND_WIDTH {
            bottom = bottom.min(height - LEGEND_RESERVE);
        }
        (top, (bottom - radius).max(top))
    };

    let pitch_budget = (line_pitch - 6.0).max(0.0);
    let near_x = layout.near_x;
    // A near-lane label is centred on the lane, so it can use the whole gap
    // between the 1-hop line and the local-network circle.
    let near_budget = (lan_rim - positions[0] - NEAR_LANE_LABEL_PAD).max(0.0);

    let mut place = |x: f64,
                     bucket: &Vec<&RemoteHost>,
                     slot_index: Option<usize>,
                     band: RemoteBand,
                     label_budget: f64| {
        if bucket.is_empty() {
            return;
        }
        let (top, bottom) = band_for(x, matches!(band, RemoteBand::Near));
        // Each line fits as many rows as its own band allows.
        let room = ((bottom - top) / slot).floor() as usize + 1;
        let shown = bucket.len().min(room.max(1));
        let height_used = (shown - 1) as f64 * slot;
        let start = (top + ((bottom - top) - height_used) / 2.0).max(top);
        for (i, host) in bucket.iter().take(shown).enumerate() {
            layout.remotes.push(PlacedRemote {
                center: slide_in(x, start + i as f64 * slot, host, now),
                radius,
                host: (*host).clone(),
                slot: slot_index,
                band,
                label_budget,
            });
        }
        layout.hidden += bucket.len() - shown;
    };

    place(layout.far_x, &far, None, RemoteBand::Far, pitch_budget);
    place(near_x, &near, None, RemoteBand::Near, near_budget);
    for (i, x) in positions.iter().enumerate() {
        place(*x, &by_slot[i], Some(i), RemoteBand::Hop(i), pitch_budget);
    }

    layout.slot = slot;
    // Labels sit centred under their circle, where they get the whole guide-line
    // pitch to work in; beside the circle there is barely any room once 20 lines
    // share the window.
    layout.label_budget = pitch_budget;
    // The per-host detail line only earns space when rows are tall enough to
    // hold it under the address.
    layout.detail_budget = if slot - radius * 2.0 >= DETAIL_ROOM {
        DETAIL_ROOM
    } else {
        0.0
    };

    layout.used = bounds(&layout, width, height);
    layout
}

fn sort_fresh(bucket: &mut Vec<&RemoteHost>) {
    bucket.sort_by(|a, b| {
        b.last_seen
            .cmp(&a.last_seen)
            .then_with(|| a.addr.cmp(&b.addr))
    });
}

/// Eases a circle in from the left-edge bucket onto its guide line.
///
/// The clock starts when the distance is first learned, so a host seen only
/// outbound waits at the edge until a reply reveals its TTL, then slides across
/// instead of teleporting.
fn slide_in(target_x: f64, target_y: f64, host: &RemoteHost, now: std::time::Instant) -> Point {
    let Some(learned) = host.hops_learned_at else {
        return Point {
            x: target_x,
            y: target_y,
        };
    };
    let elapsed = now
        .checked_duration_since(learned)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    let t = (elapsed / HOP_SLIDE_SECONDS).clamp(0.0, 1.0);
    if t >= 1.0 {
        return Point {
            x: target_x,
            y: target_y,
        };
    }
    // ease-out cubic: leaves the edge briskly, settles onto the line.
    let eased = 1.0 - (1.0 - t).powi(3);
    Point {
        x: FAR_LEFT_MARGIN + (target_x - FAR_LEFT_MARGIN) * eased,
        y: target_y,
    }
}

/// The host panel: a tall rectangle in the upper right, one square per address.
fn place_host(width: f64, height: f64, network: &LocalNetwork) -> PlacedHost {
    let top = HOST_TOP_MARGIN;
    let pad = 14.0;
    let gap = 10.0;
    let title_h = 30.0;
    // Every address assigned to the host gets a square, loopback included.
    let addresses = network.addresses();
    let count = addresses.len();

    // The panel is a vertical rectangle: it grows down the right edge, and
    // squares shrink until every address fits. Those that still do not fit are
    // dropped rather than drawn unreadably small.
    let available = (height * 0.46).max(140.0).min(height - top - 8.0);
    let mut placed = count;
    let mut square;
    loop {
        if placed == 0 {
            square = MAX_SQUARE;
            break;
        }
        let room = (available - title_h - pad * 2.0 + gap) / placed as f64 - gap;
        square = room.clamp(MIN_SQUARE, MAX_SQUARE);
        if square >= MIN_LABELED_SQUARE || placed == 1 {
            break;
        }
        placed -= 1;
    }

    let label_room = HOST_LABEL_WIDTH.min((width - 120.0).max(40.0));
    let stack_h = placed as f64 * (square + gap) - gap;
    let body_h = title_h + pad * 2.0 + stack_h.max(0.0);
    // Keep the panel taller than it is wide even with one or two addresses.
    let panel_width = pad * 2.0 + square + gap + label_room;
    let panel_height = body_h.max(panel_width * 1.2).min(available);

    let right = width - HOST_RIGHT_MARGIN;
    let left = right - panel_width;

    // Centre the stack in the panel when the panel is taller than it needs to be.
    let used_body = (panel_height - title_h - pad * 2.0).max(stack_h);
    let stack_top = top + title_h + pad + (used_body - stack_h) / 2.0;

    let mut interfaces = Vec::with_capacity(placed);
    for (i, addr) in addresses.iter().take(placed).enumerate() {
        let y = stack_top + i as f64 * (square + gap);
        interfaces.push(PlacedInterface {
            addr: addr.ip,
            iface: addr.iface.clone(),
            kind: addr.kind,
            rect: Rect::new(left + pad, y, left + pad + square, y + square),
        });
    }

    PlacedHost {
        rect: Rect::new(left, top, right, top + panel_height),
        interfaces,
        label_budget: label_room - 8.0,
    }
}

/// The local-network circle: large, in the lower-right corner.
///
/// `line_one` is the x of the 1-hop guide line. The circle may not grow past
/// `line_one - LAN_LANE_RESERVE`, because the space between the two is where
/// broadcast and multicast groups live.
fn place_lan(
    width: f64,
    height: f64,
    panel_bottom: f64,
    lan: &[LanHost],
    line_one: f64,
) -> PlacedLan {
    let mut ordered: Vec<&LanHost> = lan.iter().collect();
    ordered.sort_by(|a, b| {
        b.last_seen
            .cmp(&a.last_seen)
            .then_with(|| a.addr.cmp(&b.addr))
    });

    // The circle sits in the lower-right corner, growing to fill the space
    // below the host panel without leaving the display.
    let vertical_room = (height - panel_bottom - LAN_TITLE_RESERVE - LAN_MARGIN) / 2.0;
    let horizontal_room = (width - LAN_MARGIN - LAN_MARGIN) / 2.0;
    // The rim sits `width - LAN_MARGIN - 2 * radius` from the left, so halve
    // the room to keep the group lane between rim and line open.
    let lane_room = (width - LAN_MARGIN - LAN_LANE_RESERVE - line_one) / 2.0;
    let radius = vertical_room
        .min(horizontal_room)
        .min(lane_room)
        .clamp(LAN_MIN_RADIUS, LAN_MAX_RADIUS)
        .min(height / 2.0)
        .min(width / 2.0);

    let center = Point {
        x: width - LAN_MARGIN - radius,
        y: height - LAN_MARGIN - radius,
    };

    let (hosts, hidden) = pack_in_circle(&ordered, radius);
    PlacedLan {
        center,
        radius,
        hosts,
        hidden,
    }
}

/// Packs LAN peer circles into a grid inside the inscribed square of the
/// circle, shrinking the pitch before dropping peers.
fn pack_in_circle(peers: &[&LanHost], radius: f64) -> (Vec<PlacedLanHost>, usize) {
    // A square inscribed in the circle keeps every grid cell, corners included,
    // inside the rim.
    let side = (radius - 10.0) * std::f64::consts::SQRT_2;
    let label_h = 15.0;

    let mut placed = peers.len();
    let (cols, rows, pitch) = loop {
        if placed == 0 {
            break (0, 0, MAX_LAN_PITCH);
        }
        let cols = (placed as f64).sqrt().ceil() as usize;
        let rows = placed.div_ceil(cols);
        let pitch = (side / cols.max(rows) as f64).min(MAX_LAN_PITCH);
        if pitch >= MIN_LAN_PITCH {
            break (cols, rows, pitch);
        }
        placed -= 1;
    };

    let labeled = pitch >= LAN_LABEL_PITCH;
    let reserved = if labeled { label_h } else { 0.0 };
    let circle_radius = ((pitch - reserved) / 2.0 * 0.82).max(5.0);

    let grid_w = cols as f64 * pitch;
    let grid_h = rows as f64 * pitch;
    let origin_x = -grid_w / 2.0;
    let origin_y = -grid_h / 2.0;

    let mut hosts = Vec::with_capacity(placed);
    for (i, peer) in peers.iter().take(placed).enumerate() {
        let (col, row) = (i % cols.max(1), i / cols.max(1));
        hosts.push(PlacedLanHost {
            addr: peer.addr,
            offset: Point {
                x: origin_x + col as f64 * pitch + pitch / 2.0,
                y: origin_y + row as f64 * pitch + pitch / 2.0 - reserved / 2.0,
            },
            radius: circle_radius,
            labeled,
        });
    }
    (hosts, peers.len() - placed)
}

fn bounds(layout: &Layout, width: f64, height: f64) -> Rect {
    let mut rect = Rect::from_points((width, height), (0.0, 0.0));
    let absorb_circle = |rect: Rect, center: Point, radius: f64| {
        rect.union(Rect::from_points(
            (center.x - radius, center.y - radius),
            (center.x + radius, center.y + radius),
        ))
    };
    if let Some(host) = &layout.host {
        rect = rect.union(host.rect);
    }
    if let Some(lan) = &layout.lan {
        rect = absorb_circle(rect, lan.center, lan.radius);
    }
    for remote in &layout.remotes {
        rect = absorb_circle(rect, remote.center, remote.radius);
    }
    rect
}

/// The square that stands for a given local address.
#[must_use]
pub fn interface_rect(layout: &Layout, addr: IpAddr) -> Option<Rect> {
    layout
        .host
        .as_ref()?
        .interfaces
        .iter()
        .find(|i| i.addr == addr)
        .map(|i| i.rect)
}

/// The point on a circle's rim closest to `target`, so links stop at the edge.
#[must_use]
pub fn rim(center: Point, radius: f64, target: Point) -> Point {
    let dx = target.x - center.x;
    let dy = target.y - center.y;
    let len = (dx * dx + dy * dy).sqrt();
    if len < f64::EPSILON {
        return center;
    }
    Point {
        x: center.x + dx / len * radius,
        y: center.y + dy / len * radius,
    }
}

/// The point where a ray from `center` toward `target` leaves the rectangle.
#[must_use]
pub fn rect_edge(center: Point, half_w: f64, half_h: f64, target: Point) -> Point {
    let dx = target.x - center.x;
    let dy = target.y - center.y;
    if dx == 0.0 && dy == 0.0 {
        return center;
    }
    let sx = if dx != 0.0 {
        half_w / dx.abs()
    } else {
        f64::INFINITY
    };
    let sy = if dy != 0.0 {
        half_h / dy.abs()
    } else {
        f64::INFINITY
    };
    let scale = sx.min(sy);
    Point {
        x: center.x + dx * scale,
        y: center.y + dy * scale,
    }
}

#[must_use]
pub fn rect_center(rect: Rect) -> Point {
    Point {
        x: (rect.x0 + rect.x1) / 2.0,
        y: (rect.y0 + rect.y1) / 2.0,
    }
}

/// Anchor for a local address: the edge of its own square, or the host panel's
/// edge when that address has no square of its own.
#[must_use]
pub fn local_anchor(layout: &Layout, addr: IpAddr, target: Point) -> Option<Point> {
    layout.host.as_ref()?;
    if let Some(rect) = interface_rect(layout, addr) {
        return Some(rect_edge(
            rect_center(rect),
            rect.width() / 2.0,
            rect.height() / 2.0,
            target,
        ));
    }
    host_anchor(layout, target)
}

/// Anchor on the host panel's edge, for host <-> local-network links.
#[must_use]
pub fn host_anchor(layout: &Layout, target: Point) -> Option<Point> {
    let host = layout.host.as_ref()?;
    Some(rect_edge(
        rect_center(host.rect),
        host.rect.width() / 2.0,
        host.rect.height() / 2.0,
        target,
    ))
}

#[must_use]
pub fn lan_anchor(layout: &Layout, target: Point) -> Option<Point> {
    let lan = layout.lan.as_ref()?;
    Some(rim(lan.center, lan.radius, target))
}

/// Label for the local-network circle: the subnet(s) the host sits in.
#[must_use]
pub fn lan_label(network: &LocalNetwork) -> String {
    let subnets = network.subnets();
    match subnets.len() {
        0 => "local network".to_string(),
        1 => format!("local network · {}", crate::net::format_subnet(&subnets[0])),
        n => format!("local network · {n} subnets"),
    }
}

/// Address label beside an interface square.
#[must_use]
pub fn describe_interface(addr: &LocalAddress) -> String {
    let kind = match addr.kind {
        AddressKind::Loopback => "loopback",
        AddressKind::LinkLocal => "link-local",
        AddressKind::Global => "global",
    };
    format!("{} ({})", crate::net::format_address(addr.ip), kind)
}

/// Squared distance from `point` to the segment `a`..`b`.
/// The point of segment `a`-`b` closest to `point`.
fn closest_point_on_segment(point: Point, a: Point, b: Point) -> Point {
    let vx = b.x - a.x;
    let vy = b.y - a.y;
    let len_sq = vx * vx + vy * vy;
    if len_sq <= f64::EPSILON {
        return a;
    }
    let t = ((point.x - a.x) * vx + (point.y - a.y) * vy) / len_sq;
    let t = t.clamp(0.0, 1.0);
    Point::new(a.x + t * vx, a.y + t * vy)
}

fn distance_sq_to_segment(point: Point, a: Point, b: Point) -> f64 {
    let near = closest_point_on_segment(point, a, b);
    (point.x - near.x).powi(2) + (point.y - near.y).powi(2)
}

/// The shape under the pointer, or `None` in empty space.
///
/// Checked in reverse draw order, so whatever is painted on top wins: a peer dot
/// inside the local-network circle reads as the peer, and a circle reads as
/// itself rather than the line running underneath it. Lines are last, and are
/// matched by distance rather than by a bounding box, since a thin diagonal line
/// has a bounding box that is mostly empty space.
#[must_use]
pub fn hit_test(layout: &Layout, at: Point) -> Option<Hit> {
    for remote in &layout.remotes {
        if (at.x - remote.center.x).powi(2) + (at.y - remote.center.y).powi(2)
            <= remote.radius.powi(2)
        {
            return Some(Hit::Remote(remote.host.addr));
        }
    }
    if let Some(lan) = &layout.lan {
        for peer in &lan.hosts {
            let centre = Point {
                x: lan.center.x + peer.offset.x,
                y: lan.center.y + peer.offset.y,
            };
            if (at.x - centre.x).powi(2) + (at.y - centre.y).powi(2) <= peer.radius.powi(2) {
                return Some(Hit::LanPeer(peer.addr));
            }
        }
    }
    if let Some(host) = &layout.host {
        for iface in &host.interfaces {
            if iface.rect.contains(at) {
                return Some(Hit::Interface(iface.addr));
            }
        }
    }
    if let Some(host) = &layout.host
        && host.rect.contains(at)
    {
        return Some(Hit::HostPanel);
    }
    if let Some(lan) = &layout.lan
        && (at.x - lan.center.x).powi(2) + (at.y - lan.center.y).powi(2) <= lan.radius.powi(2)
    {
        return Some(Hit::LanCircle);
    }
    // Nearest line within a few pixels of it, widened by the line's own stroke
    // so a fat, busy link is no harder to hit than a thin one.
    let mut best: Option<(f64, LinkKey)> = None;
    for link in &layout.links {
        let reach = LINK_HIT_TOLERANCE + link.width / 2.0;
        let dist = distance_sq_to_segment(at, link.a, link.b);
        if dist <= reach * reach && best.is_none_or(|(d, _)| dist < d) {
            best = Some((dist, link.key));
        }
    }
    best.map(|(_, key)| Hit::Link(key))
}

/// Bounding box of a circle.
fn circle_box(center: Point, radius: f64) -> Rect {
    Rect::new(
        center.x - radius,
        center.y - radius,
        center.x + radius,
        center.y + radius,
    )
}

/// The box a tooltip should be placed beside, so it points at what it describes.
///
/// `at` is where the pointer is. For a line it decides the anchor: the box sits
/// on the point of the line nearest the pointer rather than spanning the whole
/// segment, so the card follows the pointer along the line instead of jumping to
/// wherever the line's bounding box happens to end.
#[must_use]
pub fn hit_anchor(layout: &Layout, hit: &Hit, at: Point) -> Option<Rect> {
    match hit {
        Hit::Remote(addr) => layout
            .remotes
            .iter()
            .find(|r| r.host.addr == *addr)
            .map(|r| circle_box(r.center, r.radius)),
        Hit::LanPeer(addr) => layout.lan.as_ref().and_then(|lan| {
            lan.hosts.iter().find(|p| p.addr == *addr).map(|p| {
                circle_box(
                    Point::new(lan.center.x + p.offset.x, lan.center.y + p.offset.y),
                    p.radius,
                )
            })
        }),
        Hit::Interface(addr) => layout
            .host
            .as_ref()
            .and_then(|h| h.interfaces.iter().find(|i| i.addr == *addr))
            .map(|i| i.rect),
        Hit::HostPanel => layout.host.as_ref().map(|h| h.rect),
        Hit::LanCircle => layout.lan.as_ref().map(|l| circle_box(l.center, l.radius)),
        Hit::Link(key) => layout.links.iter().find(|l| l.key == *key).map(|l| {
            let near = closest_point_on_segment(at, l.a, l.b);
            let half = (l.width / 2.0).max(2.0);
            Rect::new(near.x - half, near.y - half, near.x + half, near.y + half)
        }),
    }
}
