//! The picture's data model: which hosts exist, where they belong, and how
//! fresh their traffic is.
//!
//! Time is always the UI's arrival clock. A remote host stays on screen for
//! [`FADE_SECONDS`] after its last packet, then fades out over
//! [`FADE_TAIL_SECONDS`] before being dropped.

use std::collections::HashMap;
use std::net::IpAddr;
use std::time::{Duration, Instant};

use crate::capture::Sample;
use crate::capture::parse::{Frame, Transport};
use crate::net::{LocalNetwork, Location};

/// Seconds a remote host stays fully visible after its last packet.
pub const FADE_SECONDS: f64 = 30.0;
/// Extra seconds spent fading to nothing before the host is removed.
pub const FADE_TAIL_SECONDS: f64 = 4.0;
/// Cap on tracked remote hosts, so a flood cannot exhaust memory.
pub const MAX_REMOTE_HOSTS: usize = 400;

/// A host seen on the network that is neither us nor a LAN peer.
#[derive(Debug, Clone)]
pub struct RemoteHost {
    pub addr: IpAddr,
    /// Arrival time of the most recent packet to or from this address.
    pub last_seen: Instant,
    pub first_seen: Instant,
    pub packets: u64,
    pub bytes: u64,
    /// Packets exchanged with LAN peers rather than with our own interfaces.
    pub lan_packets: u64,
    pub last_transport: Transport,
}

/// A LAN peer that has been seen talking to us or to another LAN peer.
#[derive(Debug, Clone)]
pub struct LanHost {
    pub addr: IpAddr,
    pub last_seen: Instant,
    pub packets: u64,
    pub bytes: u64,
}

/// One observed endpoint pair, kept so links can be drawn and aged.
#[derive(Debug, Clone)]
pub struct Flow {
    pub remote: IpAddr,
    /// The local side: either one of our addresses or `None` when the peer was
    /// reached through the LAN circle.
    pub local: Option<IpAddr>,
    pub last_seen: Instant,
    pub packets: u64,
}

/// A LAN peer seen talking directly to one of our interfaces. Drawn as a link
/// between the interface ring and the local-network circle.
#[derive(Debug, Clone)]
pub struct LanLink {
    pub peer: IpAddr,
    pub local: IpAddr,
    pub last_seen: Instant,
    pub packets: u64,
}

#[derive(Debug, Default)]
pub struct Model {
    remotes: HashMap<IpAddr, RemoteHost>,
    lan: HashMap<IpAddr, LanHost>,
    lan_links: HashMap<(IpAddr, IpAddr), LanLink>,
    flows: HashMap<(IpAddr, Option<IpAddr>), Flow>,
    pub frames_seen: u64,
    pub frames_decoded: u64,
}

impl Model {
    /// Folds one sample into the model.
    ///
    /// Each endpoint is handled on its own, so a remote <-> remote frame
    /// produces a circle for both sides.
    pub fn ingest(&mut self, sample: &Sample, network: &LocalNetwork, now: Instant) {
        self.frames_decoded += 1;
        let frame = sample.frame;
        let src = network.classify(frame.src);
        let dst = network.classify(frame.dst);

        for (addr, location, peer, peer_location) in [
            (frame.src, src, frame.dst, dst),
            (frame.dst, dst, frame.src, src),
        ] {
            match location {
                Location::Remote => {
                    self.note_remote(addr, peer_location, Some(peer), network, now, &frame);
                }
                Location::LocalNetwork => {
                    self.note_lan(addr, now, &frame);
                    // A peer on our subnet that talked straight to one of our
                    // interfaces earns its own interface<->LAN-circle link.
                    if peer_location == Location::Host {
                        self.note_lan_link(addr, peer, now);
                    }
                }
                Location::Host => {}
            }
        }
    }

    /// Records a remote host and the flow that connects it to the local side.
    fn note_remote(
        &mut self,
        remote: IpAddr,
        peer_location: Location,
        peer_addr: Option<IpAddr>,
        network: &LocalNetwork,
        now: Instant,
        frame: &Frame,
    ) {
        let host = self.remotes.entry(remote).or_insert_with(|| RemoteHost {
            addr: remote,
            last_seen: now,
            first_seen: now,
            packets: 0,
            bytes: 0,
            lan_packets: 0,
            last_transport: frame.transport,
        });
        host.last_seen = now;
        host.packets += 1;
        host.bytes += u64::from(frame.wire_len);
        host.last_transport = frame.transport;
        if peer_location == Location::LocalNetwork {
            host.lan_packets += 1;
        }

        // The local end of the link: one of our interfaces when it is directly
        // involved, otherwise the LAN circle (represented by `None`).
        let local = match peer_location {
            Location::Host => peer_addr.filter(|a| network.is_host_address(*a)),
            Location::LocalNetwork => {
                if let Some(peer) = peer_addr {
                    self.note_lan(peer, now, frame);
                }
                None
            }
            // remote <-> remote with no local side: anchor on the LAN circle.
            Location::Remote => None,
        };

        let flow = self.flows.entry((remote, local)).or_insert_with(|| Flow {
            remote,
            local,
            last_seen: now,
            packets: 0,
        });
        flow.last_seen = now;
        flow.packets += 1;
    }

    fn note_lan_link(&mut self, peer: IpAddr, local: IpAddr, now: Instant) {
        let link = self
            .lan_links
            .entry((peer, local))
            .or_insert_with(|| LanLink {
                peer,
                local,
                last_seen: now,
                packets: 0,
            });
        link.last_seen = now;
        link.packets += 1;
    }

    fn note_lan(&mut self, addr: IpAddr, now: Instant, frame: &Frame) {
        let host = self.lan.entry(addr).or_insert_with(|| LanHost {
            addr,
            last_seen: now,
            packets: 0,
            bytes: 0,
        });
        host.last_seen = now;
        host.packets += 1;
        host.bytes += u64::from(frame.wire_len);
        let _ = frame;
    }

    /// Drops hosts and flows whose fade window has fully elapsed.
    pub fn prune(&mut self, now: Instant) {
        let horizon = Duration::from_secs_f64(FADE_SECONDS + FADE_TAIL_SECONDS);
        self.remotes
            .retain(|_, h| now.duration_since(h.last_seen) < horizon);
        self.lan
            .retain(|_, h| now.duration_since(h.last_seen) < horizon);
        self.lan_links
            .retain(|_, l| now.duration_since(l.last_seen) < horizon);
        self.flows
            .retain(|_, f| now.duration_since(f.last_seen) < horizon);
    }

    /// Evicts the stalest remotes once the cap is hit.
    pub fn enforce_cap(&mut self) {
        if self.remotes.len() <= MAX_REMOTE_HOSTS {
            return;
        }
        let mut ordered: Vec<(Instant, IpAddr)> = self
            .remotes
            .values()
            .map(|h| (h.last_seen, h.addr))
            .collect();
        ordered.sort_unstable();
        let excess = self.remotes.len() - MAX_REMOTE_HOSTS;
        for (_, addr) in ordered.into_iter().take(excess) {
            self.remotes.remove(&addr);
            self.flows.retain(|k, _| k.0 != addr);
        }
    }

    /// 1.0 while fresh, ramping to 0.0 across the fade window.
    #[must_use]
    pub fn opacity(last_seen: Instant, now: Instant) -> f32 {
        let age = now.duration_since(last_seen).as_secs_f64();
        if age <= FADE_SECONDS {
            1.0
        } else {
            let over = age - FADE_SECONDS;
            (1.0 - over / FADE_TAIL_SECONDS).clamp(0.0, 1.0) as f32
        }
    }

    pub fn remotes(&self) -> impl Iterator<Item = &RemoteHost> {
        self.remotes.values()
    }

    #[must_use]
    pub fn remote_count(&self) -> usize {
        self.remotes.len()
    }

    pub fn lan(&self) -> impl Iterator<Item = &LanHost> {
        self.lan.values()
    }

    #[must_use]
    pub fn lan_count(&self) -> usize {
        self.lan.len()
    }

    pub fn flows(&self) -> impl Iterator<Item = &Flow> {
        self.flows.values()
    }

    pub fn lan_links(&self) -> impl Iterator<Item = &LanLink> {
        self.lan_links.values()
    }

    /// Seconds since the most recent packet anywhere in the model.
    #[must_use]
    pub fn quiet_for(&self, now: Instant) -> f64 {
        self.remotes
            .values()
            .map(|h| h.last_seen)
            .chain(self.lan.values().map(|h| h.last_seen))
            .map(|t| now.duration_since(t).as_secs_f64())
            .fold(f64::INFINITY, f64::min)
    }
}
