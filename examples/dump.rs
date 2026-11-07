//! Renders one frame from a pcap savefile to a PNG for visual inspection.
//!
//!   cargo run --release --example dump -- out.png fixtures/sample.pcap \
//!       --local eth0/192.168.1.20/24

use netscope::capture::Sample;
use netscope::capture::parse::{self, LinkLayer};
use netscope::model::Model;
use netscope::net::LocalNetwork;
use netscope::render::{Hud, Scene};
use pcap::Capture;
use std::time::Instant;

const DEFAULT_WIDTH: u16 = 1920;
const DEFAULT_HEIGHT: u16 = 800;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let out = args
        .next()
        .ok_or("usage: dump OUT.png IN.pcap [--local SPECS..] [--size W H]")?;
    let input = args.next().ok_or("missing pcap path")?;
    let mut local = Vec::new();
    let mut width = DEFAULT_WIDTH;
    let mut height = DEFAULT_HEIGHT;
    let mut rest: Vec<String> = args.collect();
    if let Some(index) = rest.iter().position(|v| v == "--local") {
        rest.remove(index);
        while rest.first().is_some_and(|v| !v.starts_with("--")) {
            local.push(rest.remove(0));
        }
    }
    if let Some(index) = rest.iter().position(|v| v == "--size") {
        rest.remove(index);
        if rest.len() >= 2 {
            width = rest.remove(0).parse::<u16>().unwrap_or(DEFAULT_WIDTH);
            height = rest.remove(0).parse::<u16>().unwrap_or(DEFAULT_HEIGHT);
        }
    }

    let network = if local.is_empty() {
        LocalNetwork::discover()?
    } else {
        LocalNetwork::parse_overrides(&local)?
    };

    let mut cap = Capture::from_file(&input)?;
    let link = LinkLayer::from_pcap_value(cap.get_datalink().0);
    let mut model = Model::default();
    let t0 = Instant::now();
    while let Ok(packet) = cap.next_packet() {
        if let Some(frame) = parse::decode(link, packet.data) {
            model.ingest(&Sample { frame }, &network, t0);
        }
    }

    let mut scene = Scene::new(width, height);
    let hud = Hud {
        source: &input,
        frames_seen: model.frames_seen,
        frames_decoded: model.frames_decoded,
        remotes: model.remote_count(),
        lan_peers: model.lan_count(),
        quiet_for: 0.0,
        fps: 0.0,
        notice: Some("static frame from savefile"),
    };
    // Replay stamps every packet at t0, so drawing at t0 would freeze the
    // slide-in at its start. Advance past it to show the settled view.
    let settled = t0 + std::time::Duration::from_secs(2);
    scene.draw(&network, &model, &hud, settled);

    let rgba: Vec<u8> = scene
        .pixmap()
        .data()
        .iter()
        .flat_map(|p| [p.r, p.g, p.b, p.a])
        .collect();
    std::fs::write(&out, encode_png(u32::from(width), u32::from(height), &rgba))?;
    println!("wrote {out} ({} remote hosts)", model.remote_count());
    Ok(())
}

/// Minimal PNG writer: RGBA8, filter 0 per scanline, stored (uncompressed)
/// deflate blocks. Enough for eyeballing a frame without a PNG dependency.
fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], body: &[u8]) {
        out.extend_from_slice(&(body.len() as u32).to_be_bytes());
        out.extend_from_slice(kind);
        out.extend_from_slice(body);
        out.extend_from_slice(&crc32(kind, body).to_be_bytes());
    }

    let mut raw = Vec::with_capacity(rgba.len() + height as usize);
    for y in 0..height as usize {
        raw.push(0); // filter type: None
        let start = y * width as usize * 4;
        raw.extend_from_slice(&rgba[start..start + width as usize * 4]);
    }

    // zlib stream of stored deflate blocks.
    let mut deflate = vec![0x78, 0x01];
    for block in raw.chunks(0xffff) {
        let last = block.len() < 0xffff;
        deflate.push(u8::from(last));
        deflate.extend_from_slice(&(block.len() as u16).to_le_bytes());
        deflate.extend_from_slice(&(!(block.len() as u16)).to_le_bytes());
        deflate.extend_from_slice(block);
    }
    deflate.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // bit depth, RGBA, no compression/filter/interlace
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &deflate);
    chunk(&mut out, b"IEND", &[]);
    out
}

/// zlib's trailing checksum (RFC 1950), not a CRC.
fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for byte in data {
        a = (a + u32::from(*byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

fn crc32(prefix: &[u8], body: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in prefix.iter().chain(body) {
        let mut index = (crc ^ u32::from(*byte)) & 0xff;
        for _ in 0..8 {
            index = if index & 1 != 0 {
                (index >> 1) ^ 0xedb8_8320
            } else {
                index >> 1
            };
        }
        crc = (crc >> 8) ^ index;
    }
    !crc
}
