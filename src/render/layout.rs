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
/// Left margin for the first column of remote hosts.
pub const LEFT_MARGIN: f64 = 96.0;
/// Vertical room kept at the top of the display for the HUD text.
pub const HUD_RESERVE: f64 = 96.0;

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

#[derive(Debug, Clone)]
pub struct PlacedRemote {
    pub center: Point,
    pub radius: f64,
    pub host: RemoteHost,
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
}

/// Places every shape for the current frame.
#[must_use]
pub fn build(
    width: f64,
    height: f64,
    network: &LocalNetwork,
    remotes: &[RemoteHost],
    lan: &[LanHost],
) -> Layout {
    let mut layout = Layout::default();
    let vertical_pad = 40.0;

    let host = place_host(width, height, network);
    let lan_circle = place_lan(width, height, host.rect.y1, lan);
    layout.host = Some(host);
    layout.lan = Some(lan_circle);

    // Remote grid: fill columns left to right, shrinking the vertical slot
    // before dropping any host. Columns must clear both right-hand shapes.
    let mut ordered: Vec<&RemoteHost> = remotes.iter().collect();
    ordered.sort_by(|a, b| {
        b.last_seen
            .cmp(&a.last_seen)
            .then_with(|| a.addr.cmp(&b.addr))
    });

    let lan = layout.lan.as_ref().unwrap();
    let right_limit = layout
        .host
        .as_ref()
        .unwrap()
        .rect
        .x0
        .min(lan.center.x - lan.radius);

    let grid_top = HUD_RESERVE + 8.0;
    let usable_height = (height - grid_top - vertical_pad).max(MIN_REMOTE_SLOT);
    let columns = {
        let room = (right_limit - 24.0 - (LEFT_MARGIN - REMOTE_RADIUS)).max(0.0);
        (room / REMOTE_COLUMN_PITCH).floor() as usize + 1
    };

    let mut slot = REMOTE_SLOT;
    let mut slots = (usable_height / slot).floor() as usize;
    if ordered.len() > slots * columns {
        let rows = ordered.len().div_ceil(columns);
        slot = (usable_height / rows as f64).clamp(MIN_REMOTE_SLOT, REMOTE_SLOT);
        slots = (usable_height / slot).floor() as usize;
    }
    slots = slots.max(1);
    let radius = (REMOTE_RADIUS * (slot / REMOTE_SLOT)).max(10.0);

    let column_height = (slots.min(ordered.len()) as f64 - 1.0) * slot;
    let top_floor = grid_top + radius;
    let top_ceiling = (height - vertical_pad - radius).max(top_floor);
    let top = (height * 0.5 - column_height / 2.0).clamp(top_floor, top_ceiling);

    for (i, host) in ordered.iter().take(slots * columns).enumerate() {
        layout.remotes.push(PlacedRemote {
            center: Point {
                x: LEFT_MARGIN + (i / slots) as f64 * REMOTE_COLUMN_PITCH,
                y: top + (i % slots) as f64 * slot,
            },
            radius,
            host: (*host).clone(),
        });
    }
    layout.hidden = ordered.len().saturating_sub(slots * columns);
    layout.slot = slot;
    layout.label_budget = REMOTE_COLUMN_PITCH - radius * 2.0 - 16.0;

    layout.used = bounds(&layout, width, height);
    layout
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
fn place_lan(width: f64, height: f64, panel_bottom: f64, lan: &[LanHost]) -> PlacedLan {
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
    let radius = vertical_room
        .min(horizontal_room)
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
