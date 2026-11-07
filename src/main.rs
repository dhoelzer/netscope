//! netscope: sniff the live network (or replay a pcap file) and draw who is
//! talking to whom with Vello CPU.

use std::num::NonZeroU32;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use netscope::capture::{CaptureEvent, CaptureHandle, CaptureStats, Source};
use netscope::model::Model;
use netscope::net::LocalNetwork;
use netscope::render::{Hud, Scene};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

const DEFAULT_WIDTH: u16 = 1280;
const DEFAULT_HEIGHT: u16 = 800;
/// Frames per second while animating fades.
const FRAME_INTERVAL: Duration = Duration::from_millis(33);

fn main() -> Result<(), netscope::Error> {
    let config = Config::from_args()?;

    if config.list_devices {
        return netscope::capture::list_devices();
    }

    if let Some(path) = &config.self_test {
        return self_test(path, &config.local);
    }

    let network = Arc::new(discover_network(&config.local)?);
    if network.addresses().is_empty() {
        eprintln!("warning: no interface addresses found; everything will be drawn as remote");
    }
    for address in network.addresses() {
        println!(
            "local: {addr:<45} iface {iface}",
            addr = netscope::net::format_address(address.ip),
            iface = address.iface
        );
    }
    for subnet in network.subnets() {
        println!("subnet: {}", netscope::net::format_subnet(subnet));
    }

    let handle = CaptureHandle::spawn(config.source.clone(), config.replay_speed)?;

    let event_loop = EventLoop::new().map_err(|e| {
        netscope::Error::NoCaptureDevice(format!("could not create event loop: {e}"))
    })?;
    event_loop.set_control_flow(ControlFlow::Wait);

    let mut app = App {
        render_state: RenderState::Suspended,
        scene: Scene::new(DEFAULT_WIDTH, DEFAULT_HEIGHT),
        network,
        model: Model::default(),
        handle: Some(handle),
        stats: CaptureStats::default(),
        notice: None,
        source_label: config.source_label(),
        last_frame: None,
        fps: 0.0,
    };

    event_loop
        .run_app(&mut app)
        .map_err(|e| netscope::Error::NoCaptureDevice(format!("event loop failed: {e}")))?;
    if let Some(handle) = app.handle.take() {
        handle.join();
    }
    Ok(())
}

struct Config {
    source: Source,
    replay_speed: f64,
    list_devices: bool,
    self_test: Option<PathBuf>,
    local: Vec<String>,
}

impl Config {
    fn from_args() -> Result<Self, netscope::Error> {
        let mut args = std::env::args().skip(1);
        let mut source = Source::Device(None);
        let mut replay_speed = 1.0;
        let mut list_devices = false;
        let mut self_test = None;
        let mut local = Vec::new();

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "-h" | "--help" => {
                    print_usage();
                    std::process::exit(0);
                }
                "--list-devices" => list_devices = true,
                "--file" | "-f" => {
                    let path = args.next().ok_or_else(|| usage("--file needs a path"))?;
                    source = Source::File(PathBuf::from(path));
                }
                "--device" | "-i" => {
                    let name = args
                        .next()
                        .ok_or_else(|| usage("--device needs an interface name"))?;
                    source = Source::Device(Some(name));
                }
                "--speed" => {
                    let raw = args.next().ok_or_else(|| usage("--speed needs a number"))?;
                    replay_speed = raw
                        .parse::<f64>()
                        .map_err(|_| usage("--speed wants a number like 4.0"))?
                        .max(0.01);
                }
                "--self-test" => {
                    self_test = Some(PathBuf::from(
                        args.next()
                            .ok_or_else(|| usage("--self-test needs a path"))?,
                    ));
                }
                "--local" => loop {
                    match args.next() {
                        Some(value) if !value.starts_with("--") => local.push(value),
                        _ => break,
                    }
                },
                other => {
                    // A bare argument that names an interface is convenient.
                    if other.starts_with('-') {
                        return Err(usage(&format!("unknown option {other}")));
                    }
                    source = Source::Device(Some(other.to_string()));
                }
            }
        }
        Ok(Self {
            source,
            replay_speed,
            list_devices,
            self_test,
            local,
        })
    }

    fn source_label(&self) -> String {
        match &self.source {
            Source::File(path) => format!("replay {}", path.display()),
            Source::Device(None) => "live capture (default device)".to_string(),
            Source::Device(Some(name)) => format!("live capture on {name}"),
        }
    }
}

fn usage(message: &str) -> netscope::Error {
    print_usage();
    netscope::Error::NoCaptureDevice(message.to_string())
}

fn print_usage() {
    println!(
        "netscope — live network visualiser (pcap + Vello CPU)

usage: netscope [OPTIONS]

  -i, --device NAME   capture on interface NAME (default: first up, non-loopback)
  -f, --file PATH     replay frames from a pcap savefile
      --speed N       replay pacing multiplier (default 1.0)
      --list-devices  print capture devices and their addresses, then exit
      --self-test P   parse pcap file P and print a summary, no window
      --local SPECS   pretend these interfaces exist, e.g.
                      --local eth0/192.168.1.20/24 (repeatable, space separated)
  -h, --help          this text

Live capture needs permission: on Linux run
  sudo setcap cap_net_raw,cap_net_admin=eip target/release/netscope
after building, or run the binary with sudo. On Windows install Npcap."
    );
}

/// Headless parse of a pcap file: no window, no GPU, works in CI.
/// Uses the operator-supplied addressing when given, otherwise the real host.
fn discover_network(local: &[String]) -> Result<LocalNetwork, netscope::Error> {
    if local.is_empty() {
        LocalNetwork::discover()
    } else {
        LocalNetwork::parse_overrides(local)
    }
}

fn self_test(path: &PathBuf, local: &[String]) -> Result<(), netscope::Error> {
    use netscope::capture::parse::{self, LinkLayer};
    use pcap::Capture;

    let mut cap = Capture::from_file(path)?;
    let link = LinkLayer::from_pcap_value(cap.get_datalink().0);
    let network = discover_network(local)?;
    let mut model = Model::default();
    let mut seen = 0u64;
    let mut decoded = 0u64;
    let started = Instant::now();

    while let Ok(packet) = cap.next_packet() {
        seen += 1;
        if let Some(frame) = parse::decode(link, packet.data) {
            decoded += 1;
            model.ingest(&netscope::capture::Sample { frame }, &network, started);
        }
    }
    println!("frames: {seen}  decoded: {decoded}");
    println!("local addresses: {}", network.addresses().len());
    for subnet in network.subnets() {
        println!("  subnet {}", netscope::net::format_subnet(subnet));
    }
    println!("remote hosts: {}", model.remote_count());
    let mut remotes: Vec<_> = model.remotes().collect();
    remotes.sort_by_key(|h| std::cmp::Reverse(h.packets));
    for host in remotes.into_iter().take(15) {
        println!(
            "  {:<40} {:>6} packets",
            netscope::net::format_address(host.addr),
            host.packets
        );
    }
    println!("lan peers: {}", model.lan_count());
    Ok(())
}

enum RenderState {
    Active {
        window: Rc<Window>,
        surface: softbuffer::Surface<Rc<Window>, Rc<Window>>,
    },
    Suspended,
}

struct App {
    render_state: RenderState,
    scene: Scene,
    network: Arc<LocalNetwork>,
    model: Model,
    handle: Option<CaptureHandle>,
    stats: CaptureStats,
    notice: Option<String>,
    source_label: String,
    last_frame: Option<Instant>,
    fps: f64,
}

impl App {
    fn drain_capture(&mut self) {
        let Some(handle) = &self.handle else { return };
        // Bounded so a flood cannot starve the frame loop.
        for _ in 0..2000 {
            match handle.receiver.try_recv() {
                Ok(CaptureEvent::Sample(sample)) => {
                    let now = Instant::now();
                    self.model.ingest(&sample, &self.network, now);
                }
                Ok(CaptureEvent::Stats(stats)) => self.stats = stats,
                Ok(CaptureEvent::Finished) => {
                    self.notice = Some("replay finished".to_string());
                    self.handle.take();
                    return;
                }
                Ok(CaptureEvent::Notice(text)) => self.notice = Some(text),
                Err(std::sync::mpsc::TryRecvError::Empty) => return,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.notice = Some("capture thread stopped".to_string());
                    self.handle.take();
                    return;
                }
            }
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if matches!(self.render_state, RenderState::Active { .. }) {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("netscope")
            .with_inner_size(PhysicalSize::new(
                u32::from(DEFAULT_WIDTH),
                u32::from(DEFAULT_HEIGHT),
            ));
        let window = Rc::new(
            event_loop
                .create_window(attrs)
                .expect("could not create window"),
        );
        let context = softbuffer::Context::new(Rc::clone(&window)).expect("softbuffer context");
        let surface =
            softbuffer::Surface::new(&context, Rc::clone(&window)).expect("softbuffer surface");
        self.render_state = RenderState::Active { window, surface };
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        // Hold only the window handle here; the surface is borrowed on demand so
        // the model can be updated in between.
        let window = match &self.render_state {
            RenderState::Active { window, .. } => Rc::clone(window),
            RenderState::Suspended => return,
        };
        if window.id() != id {
            return;
        }

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                let width = size.width.clamp(1, u32::from(u16::MAX));
                let height = size.height.clamp(1, u32::from(u16::MAX));
                if self.resize_surface(width, height) {
                    self.scene.resize(width as u16, height as u16);
                    window.request_redraw();
                }
            }
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        logical_key,
                        state: ElementState::Pressed,
                        ..
                    },
                ..
            } => match logical_key {
                Key::Named(NamedKey::Escape) => event_loop.exit(),
                Key::Character(k) if k == "q" || k == "Q" => event_loop.exit(),
                Key::Named(NamedKey::Space) => self.rediscover(&window),
                _ => {}
            },
            WindowEvent::RedrawRequested => {
                let now = Instant::now();
                if let Some(last) = self.last_frame {
                    let dt = now.duration_since(last).as_secs_f64();
                    if dt > 0.0 {
                        self.fps = self.fps * 0.8 + (1.0 / dt) * 0.2;
                    }
                }
                self.last_frame = Some(now);

                self.drain_capture();
                self.model.prune(now);
                self.model.enforce_cap();
                self.model.enforce_flow_cap();

                let hud = Hud {
                    source: &self.source_label,
                    frames_seen: self.stats.seen,
                    frames_decoded: self.stats.decoded,
                    remotes: self.model.remote_count(),
                    lan_peers: self.model.lan_count(),
                    quiet_for: self.model.quiet_for(now),
                    fps: self.fps,
                    notice: self.notice.as_deref(),
                };
                self.scene.draw(&self.network, &self.model, &hud, now);
                self.present();
                window.set_title(&format!(
                    "netscope — {} remote hosts",
                    self.model.remote_count()
                ));

                // Keep a steady cadence so fades animate.
                event_loop.set_control_flow(ControlFlow::WaitUntil(now + FRAME_INTERVAL));
                window.request_redraw();
            }
            _ => {}
        }
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        self.render_state = RenderState::Suspended;
    }
}

impl App {
    fn rediscover(&mut self, window: &Window) {
        match LocalNetwork::discover() {
            Ok(fresh) => {
                self.network = Arc::new(fresh);
                self.notice = Some("interfaces re-discovered".to_string());
            }
            Err(e) => self.notice = Some(format!("re-discovery failed: {e}")),
        }
        window.request_redraw();
    }

    fn resize_surface(&mut self, width: u32, height: u32) -> bool {
        let RenderState::Active { surface, .. } = &mut self.render_state else {
            return false;
        };
        let (Some(width), Some(height)) = (NonZeroU32::new(width), NonZeroU32::new(height)) else {
            return false;
        };
        surface.resize(width, height).is_ok()
    }

    fn present(&mut self) {
        let RenderState::Active { surface, .. } = &mut self.render_state else {
            return;
        };
        let Ok(mut buffer) = surface.buffer_mut() else {
            return;
        };
        // Vello CPU writes premultiplied RGBA8; softbuffer wants 0xAARRGGBB.
        for (dst, src) in buffer.iter_mut().zip(self.scene.pixmap().data()) {
            *dst = u32::from_le_bytes([src.b, src.g, src.r, 0]);
        }
        let _ = buffer.present();
    }
}
