//! The picture's data model: which hosts exist, where they belong, and how
//! fresh their traffic is.
//!
//! Time is always the UI's arrival clock, on which two separate timers run: a
//! host circle stays fully visible for [`FADE_SECONDS`] after its last packet
//! and then fades over [`FADE_TAIL_SECONDS`], while a traffic line starts fading
//! the moment it is drawn and is gone after [`LINK_FADE_SECONDS`]. Either timer
//! is reset to full brightness by the next packet on that host or connection.

use std::collections::HashMap;
use std::net::IpAddr;
use std::time::{Duration, Instant};

use crate::capture::Sample;
use crate::capture::parse::{Frame, Transport};
use crate::net::{LocalNetwork, Location};
use crate::service::Service;

/// Seconds a remote host stays fully visible after its last packet.
pub const FADE_SECONDS: f64 = 30.0;
/// Extra seconds spent fading to nothing before the host is removed.
pub const FADE_TAIL_SECONDS: f64 = 4.0;
/// Seconds a traffic line takes to fade from full brightness to nothing. Unlike
/// a host circle, a line begins fading as soon as it is drawn.
pub const LINK_FADE_SECONDS: f64 = 45.0;
/// Cap on tracked remote hosts, so a flood cannot exhaust memory.
pub const MAX_REMOTE_HOSTS: usize = 400;
/// Cap on tracked connections, which are keyed per service as well as per host.
pub const MAX_FLOWS: usize = 2_000;
/// Hop counts a remote host can be placed at. A path longer than this is not
/// resolved to a line, and a TTL that implies more than this is unreadable.
pub const MAX_HOPS: u32 = 32;
/// Plausible initial TTLs, shortest first. The smallest one that exceeds the
/// arriving TTL and leaves at most [`MAX_HOPS`] hops is the one we assume.
///
/// The gaps between these values (64 and 127) both exceed `MAX_HOPS`, so at most
/// one candidate can ever be valid: the inference is never ambiguous.
pub const INITIAL_TTLS: [u32; 3] = [64, 128, 255];

/// Hops a packet travelled, inferred from the TTL it arrived with.
///
/// Returns `None` when no plausible initial TTL explains the value within
/// [`MAX_HOPS`] -- the host is either further away than we model or the value is
/// not worth trusting.
#[must_use]
pub fn infer_hops(ttl: u8) -> Option<u32> {
    let ttl = u32::from(ttl);
    for start in INITIAL_TTLS {
        if ttl > start {
            continue;
        }
        let hops = start - ttl;
        if hops <= MAX_HOPS {
            // A directly connected host arrives with its initial TTL intact,
            // which is 0 hops by arithmetic but one hop on the display.
            return Some(hops.max(1));
        }
    }
    None
}

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
    /// Hops from the local network, inferred from an inbound packet's TTL.
    /// `None` until a packet arrives *from* this host, which is the only kind
    /// of packet that carries a TTL worth reading.
    pub hops: Option<u32>,
    /// When the distance became known *after* the host was already on screen,
    /// which is what starts the slide from the left edge onto its guide line.
    /// `None` when the first packet already revealed it, so the host is placed
    /// on its line at once with nothing to animate.
    pub hops_learned_at: Option<Instant>,
    /// A broadcast or multicast group rather than a single system. Its TTL is
    /// meaningless as a distance, so it is placed beside the local network
    /// instead of on the hop ruler.
    pub group: bool,
}

/// A LAN peer that has been seen talking to us or to another LAN peer.
#[derive(Debug, Clone)]
pub struct LanHost {
    pub addr: IpAddr,
    pub last_seen: Instant,
    pub packets: u64,
    pub bytes: u64,
}

/// Identifies one traffic line: the two endpoints plus the connection they use.
pub type FlowKey = (IpAddr, Option<IpAddr>, Service);
/// Identifies one host-panel-to-LAN-circle line.
pub type LanLinkKey = (IpAddr, IpAddr, Service);

/// One observed endpoint pair, kept so links can be drawn and aged.
#[derive(Debug, Clone)]
pub struct Flow {
    pub remote: IpAddr,
    /// The local side: either one of our addresses or `None` when the peer was
    /// reached through the LAN circle.
    pub local: Option<IpAddr>,
    pub last_seen: Instant,
    pub packets: u64,
    /// Traffic counted by direction, keyed on the `remote` endpoint so the
    /// totals stay meaningful even when no interface of ours is involved.
    pub to_remote_packets: u64,
    pub to_remote_bytes: u64,
    pub from_remote_packets: u64,
    pub from_remote_bytes: u64,
    /// The connection this line represents: protocol plus service port. It
    /// decides the line's colour and keeps the two directions of one
    /// connection on a single line.
    pub service: Service,
}

/// A LAN peer seen talking directly to one of our interfaces. Drawn as a link
/// between the interface ring and the local-network circle.
#[derive(Debug, Clone)]
pub struct LanLink {
    pub peer: IpAddr,
    pub local: IpAddr,
    pub last_seen: Instant,
    pub packets: u64,
    pub to_peer_packets: u64,
    pub to_peer_bytes: u64,
    pub from_peer_packets: u64,
    pub from_peer_bytes: u64,
    pub service: Service,
}

/// The port that names the service on a link, so both directions of one
/// connection land on the same line and get the same colour.
///
/// Well-known ports win when only one side uses one; otherwise the smaller port
/// is taken, which keeps an ephemeral-to-ephemeral pair stable in both
/// directions. `None` when the protocol has no ports (ICMP, or a non-first
/// fragment whose transport header was never parsed).
#[must_use]
fn service_port(frame: &Frame) -> Option<u16> {
    let (a, b) = (frame.src_port?, frame.dst_port?);
    match (a < 1024, b < 1024) {
        (true, false) => Some(a),
        (false, true) => Some(b),
        _ => Some(a.min(b)),
    }
}

#[derive(Debug, Default)]
pub struct Model {
    remotes: HashMap<IpAddr, RemoteHost>,
    lan: HashMap<IpAddr, LanHost>,
    lan_links: HashMap<LanLinkKey, LanLink>,
    flows: HashMap<FlowKey, Flow>,
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
                // Remote hosts and broadcast/multicast groups both get a circle
                // and a link; `note_remote` tells them apart. See
                // `LocalNetwork::is_local_group`.
                Location::Remote | Location::LocalGroup => {
                    self.note_remote(addr, peer_location, Some(peer), network, now, &frame);
                }
                Location::LocalNetwork => {
                    self.note_lan(addr, now, &frame);
                    // A peer on our subnet that talked straight to one of our
                    // interfaces earns its own interface<->LAN-circle link.
                    if peer_location == Location::Host {
                        self.note_lan_link(addr, peer, now, &frame);
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
        let group = network.is_local_group(remote);
        let host = self.remotes.entry(remote).or_insert_with(|| RemoteHost {
            addr: remote,
            last_seen: now,
            first_seen: now,
            packets: 0,
            bytes: 0,
            lan_packets: 0,
            last_transport: frame.transport,
            hops: None,
            hops_learned_at: None,
            group,
        });
        // A host created by this very frame has not been on screen at the edge,
        // so a distance read from the same frame needs no animation.
        let existed_before = host.packets > 0;
        host.last_seen = now;
        host.packets += 1;
        host.bytes += u64::from(frame.wire_len);
        host.last_transport = frame.transport;
        // Only a packet arriving *from* the remote carries a TTL that says
        // something about distance; one we sent carries a TTL we chose.
        let learned = (!group && frame.src == remote)
            .then(|| infer_hops(frame.ttl))
            .flatten()
            .filter(|h| Some(*h) != host.hops);
        if let Some(hops) = learned {
            host.hops = Some(hops);
            // A host already sitting at the left edge has somewhere to slide
            // from; one whose first packet was inbound is placed on its line.
            if existed_before {
                host.hops_learned_at = Some(now);
            }
        }
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
            // A group on the far side: anchor on the LAN circle too, so the
            // link leaves the local-network side rather than an interface.
            Location::LocalGroup | Location::Remote => None,
        };

        let service = Service::new(frame.transport, service_port(frame));
        let flow = self
            .flows
            .entry((remote, local, service))
            .or_insert_with(|| Flow {
                remote,
                local,
                last_seen: now,
                packets: 0,
                to_remote_packets: 0,
                to_remote_bytes: 0,
                from_remote_packets: 0,
                from_remote_bytes: 0,
                service,
            });
        flow.last_seen = now;
        flow.packets += 1;
        let bytes = u64::from(frame.wire_len);
        if frame.dst == remote {
            flow.to_remote_packets += 1;
            flow.to_remote_bytes += bytes;
        } else {
            flow.from_remote_packets += 1;
            flow.from_remote_bytes += bytes;
        }
    }

    fn note_lan_link(&mut self, peer: IpAddr, local: IpAddr, now: Instant, frame: &Frame) {
        let service = Service::new(frame.transport, service_port(frame));
        let link = self
            .lan_links
            .entry((peer, local, service))
            .or_insert_with(|| LanLink {
                peer,
                local,
                last_seen: now,
                packets: 0,
                to_peer_packets: 0,
                to_peer_bytes: 0,
                from_peer_packets: 0,
                from_peer_bytes: 0,
                service,
            });
        link.last_seen = now;
        link.packets += 1;
        let bytes = u64::from(frame.wire_len);
        if frame.dst == peer {
            link.to_peer_packets += 1;
            link.to_peer_bytes += bytes;
        } else {
            link.from_peer_packets += 1;
            link.from_peer_bytes += bytes;
        }
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

    /// Drops hosts and links whose fade window has fully elapsed. Circles and
    /// lines run on different timers, so they are pruned on different horizons.
    pub fn prune(&mut self, now: Instant) {
        let host_horizon = Duration::from_secs_f64(FADE_SECONDS + FADE_TAIL_SECONDS);
        let link_horizon = Duration::from_secs_f64(LINK_FADE_SECONDS);
        self.remotes
            .retain(|_, h| now.duration_since(h.last_seen) < host_horizon);
        self.lan
            .retain(|_, h| now.duration_since(h.last_seen) < host_horizon);
        self.lan_links
            .retain(|_, l| now.duration_since(l.last_seen) < link_horizon);
        self.flows
            .retain(|_, f| now.duration_since(f.last_seen) < link_horizon);
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

    /// Evicts the stalest connections once the cap is hit. Connections are keyed
    /// per service, so a single noisy host cannot crowd out the whole picture.
    pub fn enforce_flow_cap(&mut self) {
        if self.flows.len() <= MAX_FLOWS {
            return;
        }
        let mut ordered: Vec<(Instant, FlowKey)> = self
            .flows
            .values()
            .map(|f| (f.last_seen, (f.remote, f.local, f.service)))
            .collect();
        ordered.sort_unstable_by_key(|(t, _)| *t);
        let excess = self.flows.len() - MAX_FLOWS;
        for (_, key) in ordered.into_iter().take(excess) {
            self.flows.remove(&key);
        }
    }

    /// Host circle opacity: 1.0 while fresh, ramping to 0.0 across the fade tail.
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

    /// Traffic line opacity: full brightness at the last packet, fading linearly
    /// to nothing over [`LINK_FADE_SECONDS`]. A new packet moves `last_seen`
    /// forward, which is what refreshes a line back to full brightness.
    #[must_use]
    pub fn link_opacity(last_seen: Instant, now: Instant) -> f32 {
        let age = now.duration_since(last_seen).as_secs_f64();
        (1.0 - age / LINK_FADE_SECONDS).clamp(0.0, 1.0) as f32
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

    /// The circle standing for one remote address, if it is being tracked.
    #[must_use]
    pub fn remote(&self, addr: IpAddr) -> Option<&RemoteHost> {
        self.remotes.get(&addr)
    }

    /// A peer drawn as a dot inside the local-network circle.
    #[must_use]
    pub fn lan_peer(&self, addr: IpAddr) -> Option<&LanHost> {
        self.lan.get(&addr)
    }

    /// One observed connection, named the way a drawn line names it.
    #[must_use]
    pub fn flow(&self, remote: IpAddr, local: Option<IpAddr>, service: Service) -> Option<&Flow> {
        self.flows.get(&(remote, local, service))
    }

    #[must_use]
    pub fn lan_link(&self, peer: IpAddr, local: IpAddr, service: Service) -> Option<&LanLink> {
        self.lan_links.get(&(peer, local, service))
    }

    /// Connections currently drawn from one of our interfaces.
    #[must_use]
    pub fn flows_on_interface(&self, addr: IpAddr) -> usize {
        self.flows
            .values()
            .filter(|flow| flow.local == Some(addr))
            .count()
    }
}
