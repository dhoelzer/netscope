//! Headless live-capture smoke test: prints what pcap sees, no window needed.
//!
//!   cargo run --release --example capture-probe -- [DEVICE] [SECONDS]
//!
//! Useful for checking that libpcap/Npcap is installed and that the process has
//! permission to sniff, before debugging the GUI.

use netscope::capture::{CaptureEvent, CaptureHandle, CaptureStats, Source};
use std::time::{Duration, Instant};

fn main() -> Result<(), netscope::Error> {
    let mut args = std::env::args().skip(1);
    let device = args.next();
    let seconds: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(5);

    let handle = CaptureHandle::spawn(Source::Device(device), 1.0)?;

    let deadline = Instant::now() + Duration::from_secs(seconds);
    let mut samples = 0;
    let mut stats = CaptureStats::default();
    while Instant::now() < deadline {
        match handle.receiver.try_recv() {
            Ok(CaptureEvent::Sample(sample)) => {
                samples += 1;
                if samples <= 5 {
                    let frame = &sample.frame;
                    println!(
                        "{} -> {}  {:?}",
                        netscope::net::format_address(frame.src),
                        netscope::net::format_address(frame.dst),
                        frame.transport
                    );
                }
            }
            Ok(CaptureEvent::Stats(s)) => stats = s,
            Ok(CaptureEvent::Notice(text)) => println!("notice: {text}"),
            Ok(CaptureEvent::Finished) => break,
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                std::thread::sleep(Duration::from_millis(50))
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                println!("capture thread stopped");
                break;
            }
        }
    }

    println!("samples: {samples}  stats: {stats:?}");
    handle.join();
    Ok(())
}
