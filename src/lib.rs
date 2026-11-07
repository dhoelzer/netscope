// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of netscope; see LICENSE.
//! netscope — a live network packet visualiser.
//!
//! Capture (pcap) feeds a model of which hosts matter, and Vello CPU draws the
//! result. The library half holds everything testable; `main.rs` is the window.

pub mod capture;
pub mod error;
pub mod model;
pub mod net;
pub mod render;
pub mod service;

pub use error::Error;
