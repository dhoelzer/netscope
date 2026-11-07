// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of netscope; see LICENSE.
//! Errors surfaced by netscope.

use std::fmt;

#[derive(Debug)]
pub enum Error {
    /// No usable capture device could be opened.
    NoCaptureDevice(String),
    /// The pcap savefile could not be read.
    Pcap(pcap::Error),
    /// Interface enumeration failed.
    Interfaces(std::io::Error),
    /// A capture thread died.
    CaptureThread,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NoCaptureDevice(detail) => {
                write!(f, "no usable capture device: {detail}")
            }
            Error::Pcap(e) => write!(f, "pcap error: {e}"),
            Error::Interfaces(e) => write!(f, "could not enumerate interfaces: {e}"),
            Error::CaptureThread => write!(f, "capture thread panicked"),
        }
    }
}

impl std::error::Error for Error {}

impl From<pcap::Error> for Error {
    fn from(e: pcap::Error) -> Self {
        Error::Pcap(e)
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Interfaces(e)
    }
}
