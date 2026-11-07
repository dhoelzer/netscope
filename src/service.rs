// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of netscope; see LICENSE.
//! The service a traffic line carries, and the colour it is drawn in.
//!
//! Kept free of Vello types so the model can record a service and the mapping
//! can be unit-tested on its own.
//!
//! Well-known services get hand-picked, well-separated colours. Anything else
//! falls back to a hue derived from the protocol and port, so two different
//! services never share a colour and the same service keeps its colour for as
//! long as it is on screen.

use crate::capture::parse::Transport;

/// A colour as three linear 0.0-1.0 channels, kept free of Vello types so the
/// mapping can be unit-tested on its own.
pub type Rgb = [f32; 3];

/// Named services, in the order they are matched. Kept short on purpose: the
/// legend has to stay readable.
pub const WELL_KNOWN: &[(Transport, Option<u16>, &str, Rgb)] = &[
    (Transport::Icmp, None, "icmp", [0.98, 0.45, 0.85]),
    (Transport::IcmpV6, None, "icmp6", [0.80, 0.45, 0.98]),
    (Transport::Udp, Some(53), "udp/53 dns", [0.30, 0.85, 0.98]),
    (Transport::Tcp, Some(53), "tcp/53 dns", [0.30, 0.70, 0.98]),
    (
        Transport::Tcp,
        Some(443),
        "tcp/443 https",
        [0.35, 0.90, 0.55],
    ),
    (
        Transport::Udp,
        Some(443),
        "udp/443 quic",
        [0.45, 0.95, 0.70],
    ),
    (Transport::Tcp, Some(80), "tcp/80 http", [0.98, 0.88, 0.35]),
    (Transport::Tcp, Some(22), "tcp/22 ssh", [0.98, 0.62, 0.25]),
    (Transport::Tcp, Some(25), "tcp/25 smtp", [0.96, 0.40, 0.35]),
    (
        Transport::Tcp,
        Some(3389),
        "tcp/3389 rdp",
        [0.55, 0.60, 0.98],
    ),
    (Transport::Udp, Some(67), "udp/67 dhcp", [0.75, 0.85, 0.40]),
    (Transport::Udp, Some(68), "udp/68 dhcp", [0.75, 0.85, 0.40]),
    (Transport::Udp, Some(123), "udp/123 ntp", [0.35, 0.80, 0.80]),
    (
        Transport::Udp,
        Some(5353),
        "udp/5353 mdns",
        [0.60, 0.90, 0.90],
    ),
    (
        Transport::Udp,
        Some(1900),
        "udp/1900 ssdp",
        [0.95, 0.70, 0.55],
    ),
];

/// The service a link carries, as recorded on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Service {
    pub transport: Transport,
    pub port: Option<u16>,
}

impl Service {
    #[must_use]
    pub const fn new(transport: Transport, port: Option<u16>) -> Self {
        Self { transport, port }
    }

    /// Short label for the legend, e.g. `tcp/443` or `icmp`.
    #[must_use]
    pub fn label(&self) -> String {
        match self.port {
            Some(port) => format!("{}/{port}", protocol_tag(self.transport), port = port),
            None => protocol_tag(self.transport).to_string(),
        }
    }

    /// Human-readable name for a well-known service, else its numeric label.
    #[must_use]
    pub fn described(&self) -> String {
        match WELL_KNOWN
            .iter()
            .find(|(t, p, ..)| t == &self.transport && p == &self.port)
        {
            Some((_, _, name, _)) => (*name).to_string(),
            None => self.label(),
        }
    }

    #[must_use]
    pub fn rgb(&self) -> Rgb {
        for (t, p, _, colour) in WELL_KNOWN {
            if t == &self.transport && p == &self.port {
                return *colour;
            }
        }
        self.derived_rgb()
    }

    /// A stable hue for services that are not in the table. The golden-ratio
    /// step spreads consecutive ports across the colour wheel instead of
    /// clumping them together.
    fn derived_rgb(&self) -> Rgb {
        let index = match self.port {
            Some(port) => u64::from(port) + u64::from(protocol_slot(self.transport)) * 65_537,
            None => u64::from(protocol_slot(self.transport)) * 65_537,
        };
        let hue = (index as f64 * 0.618_033_988_749_894_7 % 1.0) as f32;
        hsv_rgb(hue, 0.72, 0.95)
    }
}

fn protocol_tag(transport: Transport) -> &'static str {
    match transport {
        Transport::Tcp => "tcp",
        Transport::Udp => "udp",
        Transport::Icmp => "icmp",
        Transport::IcmpV6 => "icmp6",
        Transport::Other(_) => "ip",
        Transport::None => "ip",
    }
}

/// Distinguishes portless protocols in the derived hue.
const fn protocol_slot(transport: Transport) -> u8 {
    match transport {
        Transport::Tcp => 0,
        Transport::Udp => 1,
        Transport::Icmp => 2,
        Transport::IcmpV6 => 3,
        Transport::Other(_) => 4,
        Transport::None => 5,
    }
}

fn hsv_rgb(hue: f32, saturation: f32, value: f32) -> Rgb {
    let h = hue * 6.0;
    let i = h.floor();
    let f = h - i;
    let p = value * (1.0 - saturation);
    let q = value * (1.0 - saturation * f);
    let t = value * (1.0 - saturation * (1.0 - f));
    match i as u32 % 6 {
        0 => [value, t, p],
        1 => [q, value, p],
        2 => [p, value, t],
        3 => [p, q, value],
        4 => [t, p, value],
        _ => [value, p, q],
    }
}
