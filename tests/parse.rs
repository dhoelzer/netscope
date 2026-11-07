// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of netscope; see LICENSE.
//! Frame-decoding tests against synthetic packets (no network access needed).

use netscope::capture::parse::{LinkLayer, Transport, decode};
use std::net::IpAddr;

fn ipv4(src: [u8; 4], dst: [u8; 4], proto: u8, payload: &[u8]) -> Vec<u8> {
    let total = 20 + payload.len();
    let mut p = vec![
        0x45,
        0,
        (total >> 8) as u8,
        total as u8,
        0,
        0,
        0x40,
        0, // flags = DF, frag offset = 0
        64,
        proto,
        0,
        0, // checksum (not validated here)
    ];
    p.extend_from_slice(&src);
    p.extend_from_slice(&dst);
    p.extend_from_slice(payload);
    p
}

fn ethernet(body: &[u8], ethertype: u16) -> Vec<u8> {
    let mut frame = vec![0xaa; 6];
    frame.extend_from_slice(&[0xbb; 6]);
    frame.extend_from_slice(&ethertype.to_be_bytes());
    frame.extend_from_slice(body);
    frame
}

fn tcp_header(src: u16, dst: u16) -> Vec<u8> {
    let mut h = Vec::new();
    h.extend_from_slice(&src.to_be_bytes());
    h.extend_from_slice(&dst.to_be_bytes());
    h.extend_from_slice(&[0; 8]);
    h.extend_from_slice(&[0x50, 0x10]);
    h.extend_from_slice(&[0xff, 0xff, 0, 0, 0, 0]); // window, checksum, urgent
    h
}

fn udp_header(src: u16, dst: u16) -> Vec<u8> {
    let mut h = Vec::new();
    h.extend_from_slice(&src.to_be_bytes());
    h.extend_from_slice(&dst.to_be_bytes());
    h.extend_from_slice(&[0, 12]);
    h.extend_from_slice(&[0, 0]);
    h
}

#[test]
fn ethernet_ipv4_tcp() {
    let body = ipv4(
        [192, 168, 1, 10],
        [93, 184, 216, 34],
        6,
        &tcp_header(1234, 443),
    );
    let frame = ethernet(&body, 0x0800);
    let d = decode(LinkLayer::Ethernet, &frame).expect("should decode");
    assert_eq!(d.src, IpAddr::from([192, 168, 1, 10]));
    assert_eq!(d.dst, IpAddr::from([93, 184, 216, 34]));
    assert_eq!(d.transport, Transport::Tcp);
    assert_eq!(d.src_port, Some(1234));
    assert_eq!(d.dst_port, Some(443));
    assert_eq!(d.wire_len, 20 + 20);
}

#[test]
fn vlan_tagged_ipv4_decodes() {
    let body = ipv4([10, 0, 0, 1], [10, 0, 0, 2], 17, &udp_header(53, 5353));
    let mut frame = vec![0xaa; 6];
    frame.extend_from_slice(&[0xbb; 6]);
    frame.extend_from_slice(&0x8100u16.to_be_bytes());
    frame.extend_from_slice(&[0x00, 0x0a]);
    frame.extend_from_slice(&0x0800u16.to_be_bytes());
    frame.extend_from_slice(&body);

    let d = decode(LinkLayer::Ethernet, &frame).expect("vlan should decode");
    assert_eq!(d.transport, Transport::Udp);
    assert_eq!(d.src_port, Some(53));
    assert_eq!(d.dst_port, Some(5353));
}

#[test]
fn raw_ip_linktype() {
    let body = ipv4([1, 2, 3, 4], [5, 6, 7, 8], 6, &tcp_header(1, 2));
    let d = decode(LinkLayer::RawIp, &body).expect("raw ip should decode");
    assert_eq!(d.src, IpAddr::from([1, 2, 3, 4]));
}

#[test]
fn null_loopback_linktype() {
    let body = ipv4([127, 0, 0, 1], [127, 0, 0, 2], 6, &tcp_header(9, 9));
    let mut frame = (2u32).to_ne_bytes().to_vec(); // AF_INET, native order
    frame.extend_from_slice(&body);
    let d = decode(LinkLayer::NullLoopback, &frame).expect("loopback should decode");
    assert_eq!(d.dst, IpAddr::from([127, 0, 0, 2]));
}

#[test]
fn linux_sll_linktype() {
    let body = ipv4([172, 16, 0, 5], [8, 8, 8, 8], 17, &udp_header(40000, 53));
    let mut frame = vec![0u8; 16];
    frame[14..16].copy_from_slice(&0x0800u16.to_be_bytes());
    frame.extend_from_slice(&body);
    let d = decode(LinkLayer::LinuxSll, &frame).expect("sll should decode");
    assert_eq!(d.dst, IpAddr::from([8, 8, 8, 8]));
    assert_eq!(d.dst_port, Some(53));
}

#[test]
fn ipv6_tcp() {
    let payload = tcp_header(8080, 80);
    let src: [u8; 16] = [0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
    let dst: [u8; 16] = [0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2];
    let mut p = vec![0x60, 0, 0, 0];
    p.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    p.extend_from_slice(&[6, 64]);
    p.extend_from_slice(&src);
    p.extend_from_slice(&dst);
    p.extend_from_slice(&payload);

    let d = decode(LinkLayer::RawIp, &p).expect("ipv6 should decode");
    assert_eq!(d.src, IpAddr::from(src));
    assert_eq!(d.dst, IpAddr::from(dst));
    assert_eq!(d.transport, Transport::Tcp);
    assert_eq!(d.src_port, Some(8080));
}

#[test]
fn ipv6_hop_by_hop_extension_is_walked() {
    let payload = tcp_header(7, 7);
    let ext = [0x06u8, 0x00, 0, 0, 0, 0, 0, 0]; // next = TCP, hdr_ext_len 0 -> 8 bytes
    let src: [u8; 16] = [0xfe, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3];
    let dst: [u8; 16] = [0xfe, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 4];
    let mut p = vec![0x60, 0, 0, 0];
    p.extend_from_slice(&((ext.len() + payload.len()) as u16).to_be_bytes());
    p.extend_from_slice(&[0, 64]); // next = hop-by-hop
    p.extend_from_slice(&src);
    p.extend_from_slice(&dst);
    p.extend_from_slice(&ext);
    p.extend_from_slice(&payload);

    let d = decode(LinkLayer::RawIp, &p).expect("extension header should be walked");
    assert_eq!(d.transport, Transport::Tcp);
    assert_eq!(d.src_port, Some(7));
    assert_eq!(d.dst_port, Some(7));
}

#[test]
fn non_ip_and_truncated_frames_are_skipped() {
    let arp = ethernet(&[0u8; 28], 0x0806);
    assert!(decode(LinkLayer::Ethernet, &arp).is_none());
    assert!(decode(LinkLayer::Ethernet, &[0u8; 10]).is_none());
    assert!(decode(LinkLayer::RawIp, &[0x45, 0, 0, 20, 0, 0]).is_none());
    assert!(decode(LinkLayer::Unsupported(228), &[0u8; 60]).is_none());
    assert!(decode(LinkLayer::RawIp, &[]).is_none());
}

#[test]
fn ipv4_options_are_skipped() {
    let mut body = ipv4([10, 0, 0, 1], [10, 0, 0, 2], 6, &tcp_header(5, 6));
    body[0] = 0x46; // IHL = 6 -> 24-byte header
    body.splice(20..20, [0x00, 0x00, 0x00, 0x00]);
    let d = decode(LinkLayer::RawIp, &body).expect("options should be skipped");
    assert_eq!(d.src_port, Some(5));
    assert_eq!(d.dst_port, Some(6));
}

#[test]
fn linktype_mapping() {
    assert_eq!(LinkLayer::from_pcap_value(1), LinkLayer::Ethernet);
    assert_eq!(LinkLayer::from_pcap_value(101), LinkLayer::RawIp);
    assert_eq!(LinkLayer::from_pcap_value(0), LinkLayer::NullLoopback);
    assert_eq!(LinkLayer::from_pcap_value(108), LinkLayer::NullLoopback);
    assert_eq!(LinkLayer::from_pcap_value(113), LinkLayer::LinuxSll);
    assert_eq!(LinkLayer::from_pcap_value(127), LinkLayer::Unsupported(127));
}

#[test]
fn ipv4_ttl_is_reported() {
    let payload = tcp_header(1, 2);
    let mut p = ipv4([192, 0, 2, 1], [192, 0, 2, 2], 6, &payload);
    p[8] = 52;
    let d = decode(LinkLayer::Ethernet, &ethernet(&p, 0x0800)).expect("decodes");
    assert_eq!(d.ttl, 52);
}

#[test]
fn ipv4_ttl_survives_header_options() {
    // IHL 6: twenty bytes of options after the fixed header. TTL is in the
    // fixed part, so it must still read from byte 8.
    let payload = tcp_header(1, 2);
    let mut p = ipv4([192, 0, 2, 1], [192, 0, 2, 2], 6, &payload);
    p[0] = 0x46;
    p.splice(20..20, std::iter::repeat_n(0x01, 4));
    p[8] = 251;
    // Total length has to account for the four extra option bytes.
    let total = (20 + 4 + payload.len()) as u16;
    p[2..4].copy_from_slice(&total.to_be_bytes());
    let d = decode(LinkLayer::Ethernet, &ethernet(&p, 0x0800)).expect("decodes");
    assert_eq!(d.ttl, 251);
}

#[test]
fn ipv6_hop_limit_is_reported_as_ttl() {
    let payload = tcp_header(8080, 80);
    let src: [u8; 16] = [0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
    let dst: [u8; 16] = [0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2];
    let mut p = vec![0x60, 0, 0, 0];
    p.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    p.extend_from_slice(&[6, 251]);
    p.extend_from_slice(&src);
    p.extend_from_slice(&dst);
    p.extend_from_slice(&payload);

    let d = decode(LinkLayer::RawIp, &p).expect("ipv6 should decode");
    assert_eq!(d.ttl, 251);
}
