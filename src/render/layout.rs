//! Where things go on screen.
//!
//! Layout is deliberately simple and deterministic: the host sits left of
//! centre, the LAN circle to its right, and remote hosts fill the left margin
//! in vertical slots ordered by how recently they were seen.

use std::net::IpAddr;

use vello_cpu::kurbo::{Point, Rect};

use crate::model::{LanHost, RemoteHost};
use crate::net::{LocalAddress, LocalNetwork};

/// Radius of the circle that stands for the host itself.
pub const HOST_RADIUS: f64 = 92.0;
/// Radius step between nested interface circles.
pub const INTERFACE_STEP: f64 = 26.0;
/// Radius of the LAN circle.
pub const LAN_RADIUS: f64 = 66.0;
/// Radius of a remote-host circle.
pub const REMOTE_RADIUS: f64 = 26.0;
/// Vertical spacing between remote-host slots when there is room for it.
pub const REMOTE_SLOT: f64 = 62.0;
/// Smallest slot still worth drawing; past this, hosts are dropped instead.
pub const MIN_REMOTE_SLOT: f64 = 34.0;
/// Horizontal spacing between remote columns.
pub const REMOTE_COLUMN_PITCH: f64 = 200.0;
/// Left margin for the first column of remote hosts.
pub const LEFT_MARGIN: f64 = 96.0;

#[derive(Debug, Clone)]
pub struct PlacedInterface {
    pub addr: IpAddr,
    pub iface: String,
    pub radius: f64,
}

#[derive(Debug, Clone)]
pub struct PlacedHost {
    pub center: Point,
    pub radius: f64,
    pub interfaces: Vec<PlacedInterface>,
}

#[derive(Debug, Clone)]
pub struct PlacedLan {
    pub center: Point,
    pub radius: f64,
    pub hosts: Vec<LanHost>,
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

/// Places every circle for the current frame.
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

    // Remote column: newest first, so the busiest hosts sit near the middle.
    let mut ordered: Vec<&RemoteHost> = remotes.iter().collect();
    ordered.sort_by(|a, b| {
        b.last_seen
            .cmp(&a.last_seen)
            .then_with(|| a.addr.cmp(&b.addr))
    });

    let host_center = Point {
        x: (width * 0.5 + LEFT_MARGIN * 0.5).min(width - LAN_RADIUS - 40.0),
        y: height * 0.5,
    };

    let interfaces: Vec<PlacedInterface> = network
        .display_addresses()
        .into_iter()
        .enumerate()
        .map(|(i, addr)| PlacedInterface {
            addr: addr.ip,
            iface: addr.iface.clone(),
            radius: (HOST_RADIUS - INTERFACE_STEP * (i as f64 + 1.0)).max(12.0),
        })
        .collect();

    layout.host = Some(PlacedHost {
        center: host_center,
        radius: HOST_RADIUS,
        interfaces,
    });

    let lan_center = Point {
        x: (host_center.x + HOST_RADIUS + LAN_RADIUS + 48.0).min(width - LAN_RADIUS - 24.0),
        y: height * 0.5,
    };
    layout.lan = Some(PlacedLan {
        center: lan_center,
        radius: LAN_RADIUS,
        hosts: lan.to_vec(),
    });

    // Remote grid: fill columns left to right, and shrink the vertical slot
    // before dropping any host.
    let usable_height = (height - vertical_pad * 2.0).max(MIN_REMOTE_SLOT);
    let columns = {
        let room = (host_center.x - HOST_RADIUS - 24.0 - (LEFT_MARGIN - REMOTE_RADIUS)).max(0.0);
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

    // Centre the grid vertically on the host circle.
    let column_height = (slots.min(ordered.len()) as f64 - 1.0) * slot;
    let top = (host_center.y - column_height / 2.0)
        .max(vertical_pad + radius)
        .max(0.0);

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

fn bounds(layout: &Layout, width: f64, height: f64) -> Rect {
    let mut rect = Rect::from_points((width, height), (0.0, 0.0));
    let mut absorb = |center: Point, radius: f64| {
        rect = rect.union(Rect::from_points(
            (center.x - radius, center.y - radius),
            (center.x + radius, center.y + radius),
        ));
    };
    if let Some(host) = &layout.host {
        absorb(host.center, host.radius);
    }
    if let Some(lan) = &layout.lan {
        absorb(lan.center, lan.radius);
    }
    for remote in &layout.remotes {
        absorb(remote.center, remote.radius);
    }
    rect
}

/// Radius of the interface circle that a given local address owns.
#[must_use]
pub fn interface_radius(layout: &Layout, addr: IpAddr) -> Option<f64> {
    layout
        .host
        .as_ref()?
        .interfaces
        .iter()
        .find(|i| i.addr == addr)
        .map(|i| i.radius)
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

/// Anchor for a local address: the rim of its interface circle, or the host
/// circle when the address has no circle of its own.
#[must_use]
pub fn local_anchor(layout: &Layout, addr: IpAddr, target: Point) -> Option<Point> {
    let host = layout.host.as_ref()?;
    match interface_radius(layout, addr) {
        Some(radius) => Some(rim(host.center, radius, target)),
        None => Some(rim(host.center, host.radius, target)),
    }
}

#[must_use]
pub fn lan_anchor(layout: &Layout, target: Point) -> Option<Point> {
    let lan = layout.lan.as_ref()?;
    Some(rim(lan.center, lan.radius, target))
}

/// Label for the middle ring: the subnet(s) the host sits in.
#[must_use]
pub fn lan_label(network: &LocalNetwork) -> String {
    let subnets = network.subnets();
    match subnets.len() {
        0 => "local network".to_string(),
        1 => crate::net::format_subnet(&subnets[0]),
        n => format!("{} local subnets", n),
    }
}

/// Which local address a label belongs to, for the interface legend.
#[must_use]
pub fn describe_interface(addr: &LocalAddress) -> String {
    let kind = match addr.kind {
        crate::net::AddressKind::Loopback => "loopback",
        crate::net::AddressKind::LinkLocal => "link-local",
        crate::net::AddressKind::Global => "global",
    };
    format!("{} ({})", crate::net::format_address(addr.ip), kind)
}
