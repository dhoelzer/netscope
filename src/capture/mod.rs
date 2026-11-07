// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of netscope; see LICENSE.
//! Packet sources: live capture on a device, or replay from a pcap savefile.
//!
//! `pcap::Capture<Active>` and `pcap::Capture<Offline>` are distinct types, so
//! both are wrapped here and drained on a dedicated thread (a `Capture` is
//! `Send` but not `Sync`). Parsed frames travel to the UI over a channel.

use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use pcap::{Capture, Device, Error as PcapError};

pub mod parse;

use crate::capture::parse::{Frame, LinkLayer};
use crate::error::Error;

/// A decoded frame delivered to the UI. Fades are always driven by the UI's
/// own arrival clock, so replayed files need no timestamp translation.
#[derive(Debug, Clone, Copy)]
pub struct Sample {
    pub frame: Frame,
}

/// What the capture thread reports alongside samples.
#[derive(Debug, Clone)]
pub enum CaptureEvent {
    Sample(Sample),
    /// Periodic counters so the HUD can show capture health.
    Stats(CaptureStats),
    /// The savefile reached EOF.
    Finished,
    /// A non-fatal problem worth surfacing to the user.
    Notice(String),
}

#[derive(Debug, Clone, Copy, Default)]
pub struct CaptureStats {
    pub seen: u64,
    pub decoded: u64,
    pub dropped: u64,
}

#[derive(Debug, Clone)]
pub enum Source {
    /// Capture on the interface with this name; `None` = pcap's default device.
    Device(Option<String>),
    /// Replay a pcap savefile.
    File(PathBuf),
}

/// Handle to a running capture. Dropping the receiver stops the thread.
pub struct CaptureHandle {
    pub receiver: Receiver<CaptureEvent>,
    thread: JoinHandle<()>,
    stop: Arc<Mutex<bool>>,
}

impl CaptureHandle {
    pub fn spawn(source: Source, speed: f64) -> Result<Self, Error> {
        let (tx, rx) = mpsc::channel::<CaptureEvent>();
        let stop = Arc::new(Mutex::new(false));
        let stop_thread = Arc::clone(&stop);
        let thread = thread::Builder::new()
            .name("netscope-capture".into())
            .spawn(move || {
                if let Err(e) = run(source, speed, tx, stop_thread) {
                    eprintln!("capture failed: {e}");
                }
            })?;
        Ok(Self {
            receiver: rx,
            thread,
            stop,
        })
    }

    pub fn join(self) {
        if let Ok(mut flag) = self.stop.lock() {
            *flag = true;
        }
        let _ = self.thread.join();
    }
}

fn should_stop(stop: &Arc<Mutex<bool>>) -> bool {
    stop.lock().map(|s| *s).unwrap_or(true)
}

fn run(
    source: Source,
    speed: f64,
    tx: Sender<CaptureEvent>,
    stop: Arc<Mutex<bool>>,
) -> Result<(), Error> {
    match source {
        Source::File(path) => run_file(&path, speed, tx, stop),
        Source::Device(name) => run_device(name, tx, stop),
    }
}

/// BPF that keeps only IP traffic; ARP/LLDP/802.11 management never reaches Rust.
const BPF: &str = "ip or ip6";

fn run_device(
    name: Option<String>,
    tx: Sender<CaptureEvent>,
    stop: Arc<Mutex<bool>>,
) -> Result<(), Error> {
    let device = match name {
        Some(name) => Device::list()?
            .into_iter()
            .find(|d| d.name == name || d.desc.as_deref() == Some(name.as_str()))
            .ok_or_else(|| {
                Error::NoCaptureDevice(format!("no device named {name:?}; try --list-devices"))
            })?,
        None => pick_default_device()?,
    };

    let mut cap = Capture::from_device(device)
        .map_err(|e| Error::NoCaptureDevice(e.to_string()))?
        .promisc(false)
        .snaplen(SNAPLEN)
        .timeout(150)
        .immediate_mode(false)
        .open()?;

    if let Err(e) = cap.filter(BPF, true) {
        tx.send(CaptureEvent::Notice(format!(
            "BPF filter not applied ({e}); decoding every frame instead"
        )))
        .ok();
    }

    let link = LinkLayer::from_pcap_value(cap.get_datalink().0);
    if matches!(link, LinkLayer::Unsupported(_)) {
        tx.send(CaptureEvent::Notice(format!(
            "unsupported link type {}; frames will be ignored",
            link_name(cap.get_datalink().0)
        )))
        .ok();
    }

    let mut stats = CaptureStats::default();
    let mut last_stats = Instant::now();
    while !should_stop(&stop) {
        match cap.next_packet() {
            Ok(packet) => {
                stats.seen += 1;
                if let Some(frame) = parse::decode(link, packet.data) {
                    stats.decoded += 1;
                    if !interesting(&frame) {
                        continue;
                    }
                    if tx.send(CaptureEvent::Sample(Sample { frame })).is_err() {
                        return Ok(());
                    }
                } else {
                    stats.dropped += 1;
                }
            }
            // A timeout just means the link was quiet; keep polling.
            Err(PcapError::TimeoutExpired) => {}
            Err(e) => return Err(e.into()),
        }
        if last_stats.elapsed() >= Duration::from_millis(500) {
            last_stats = Instant::now();
            if tx.send(CaptureEvent::Stats(stats)).is_err() {
                return Ok(());
            }
        }
    }
    Ok(())
}

const SNAPLEN: i32 = 128;

fn run_file(
    path: &Path,
    speed: f64,
    tx: Sender<CaptureEvent>,
    stop: Arc<Mutex<bool>>,
) -> Result<(), Error> {
    let mut cap = Capture::from_file(path)?;
    let link = LinkLayer::from_pcap_value(cap.get_datalink().0);
    let started = Instant::now();
    let mut first_ts: Option<i64> = None;

    let mut stats = CaptureStats::default();
    while !should_stop(&stop) {
        match cap.next_packet() {
            Ok(packet) => {
                stats.seen += 1;
                if let Some(frame) = parse::decode(link, packet.data) {
                    stats.decoded += 1;
                    if interesting(&frame) {
                        // Replay the file at its original cadence so the fade
                        // behaviour is observable; `speed` compresses the gap.
                        let ts = packet.header.ts.tv_sec;
                        let base = *first_ts.get_or_insert(ts);
                        let rel = (i64::max(ts - base, 0) as f64) / speed.max(0.001);
                        if tx.send(CaptureEvent::Sample(Sample { frame })).is_err() {
                            return Ok(());
                        }
                        pace(started, rel, &stop);
                    }
                } else {
                    stats.dropped += 1;
                }
            }
            Err(PcapError::NoMorePackets) => {
                tx.send(CaptureEvent::Stats(stats)).ok();
                tx.send(CaptureEvent::Finished).ok();
                return Ok(());
            }
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

/// Sleeps until `started + rel`, in short slices so `--stop` stays responsive.
fn pace(started: Instant, rel: f64, stop: &Arc<Mutex<bool>>) {
    let target = started + Duration::from_secs_f64(rel.min(MAX_REPLAY_GAP));
    while target > Instant::now() && !should_stop(stop) {
        thread::sleep(Duration::from_millis(20));
    }
}

/// Longest pause a replay will honour between two packets, in seconds.
const MAX_REPLAY_GAP: f64 = 5.0;

/// Skips frames where neither endpoint matters to the picture (e.g. two
/// loopback endpoints talking to each other).
fn interesting(frame: &Frame) -> bool {
    let bad = |ip: IpAddr| match ip {
        IpAddr::V4(v4) => v4.is_unspecified() || v4.is_multicast() || v4.is_loopback(),
        IpAddr::V6(v6) => v6.is_unspecified() || v6.is_loopback(),
    };
    !(bad(frame.src) && bad(frame.dst))
}

fn link_name(value: i32) -> String {
    pcap::Linktype(value)
        .get_name()
        .unwrap_or_else(|_| "unknown".to_string())
}

/// Chooses the most plausible interface when the user did not name one:
/// the first up+running non-loopback device that has an IPv4 address.
fn pick_default_device() -> Result<Device, Error> {
    let devices = Device::list()?;
    let mut fallback: Option<Device> = None;
    for device in devices {
        if device.flags.is_loopback() || !device.flags.is_up() {
            continue;
        }
        // libpcap pseudo-devices that cannot be opened normally.
        if matches!(device.name.as_str(), "any" | "linux-virtual") {
            continue;
        }
        if device.addresses.iter().any(|a| a.addr.is_ipv4()) {
            return Ok(device);
        }
        fallback.get_or_insert(device);
    }
    fallback.ok_or_else(|| Error::NoCaptureDevice("no up, non-loopback device found".into()))
}

/// Prints every device pcap can see, for `--list-devices`.
pub fn list_devices() -> Result<(), Error> {
    for device in Device::list()? {
        let flags = &device.flags;
        println!(
            "{}  up={} running={} loopback={}{}",
            device.name,
            flags.is_up(),
            flags.is_running(),
            flags.is_loopback(),
            device
                .desc
                .as_ref()
                .map(|d| format!("  ({d})"))
                .unwrap_or_default()
        );
        for address in &device.addresses {
            let mask = address
                .netmask
                .map(|m| format!("/{}", m))
                .unwrap_or_default();
            println!("    {addr}{mask}", addr = address.addr);
        }
    }
    Ok(())
}
