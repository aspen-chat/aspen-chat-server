//! The mediasoup settings every router and transport shares: the codecs a router offers, the
//! H.264 parameters game capture sends, how a client's WebRTC transport listens, where a plain
//! transport listens, and the conversion between mediasoup's media kinds and the signalling
//! protocol's.

use crate::config::MediaAddress;
use mediasoup::prelude::*;
use mediasoup::types::data_structures::{SocketFlags, TransportTuple};
use std::num::{NonZeroU8, NonZeroU32};
use voice_protocol::signal::MediaKind as WireKind;

/// What a transport assumes a participant can receive before it has measured, in bits per
/// second: enough for a screen share at full quality from its first seconds. The voice server
/// assumes its network can carry the best picture and lets each receiver's own bandwidth
/// estimate bring it down, rather than starting low (mediasoup's default is 600 kbps) and
/// making every share blurry while the estimate climbs.
const INITIAL_OUTGOING_BITRATE: u64 = 10_000_000;

/// How a transport listens on `address` over `protocol`, on a port of the worker's range.
pub(crate) fn listen_info(address: &MediaAddress, protocol: Protocol) -> ListenInfo {
    ListenInfo {
        protocol,
        ip: address.ip,
        announced_address: address.announced.clone(),
        expose_internal_ip: false,
        port: None,
        port_range: None,
        // An IPv6 socket takes IPv6 alone, so each port of the range serves both families,
        // one socket each, and the range carries as many people as it does with one family.
        flags: address.ip.is_ipv6().then_some(SocketFlags {
            ipv6_only: true,
            udp_reuse_port: false,
        }),
        send_buffer_size: None,
        recv_buffer_size: None,
    }
}

/// How a client's WebRTC transport listens: on each of `addresses` (never empty), over UDP and
/// TCP, UDP preferred. The client's ICE picks the one it reaches.
pub(crate) fn webrtc_transport_options(addresses: &[MediaAddress]) -> WebRtcTransportOptions {
    let mut listen_infos = [Protocol::Udp, Protocol::Tcp]
        .into_iter()
        .flat_map(|protocol| {
            addresses
                .iter()
                .map(move |address| listen_info(address, protocol))
        });
    let first = listen_infos.next().expect("a media address");
    let listen_infos = listen_infos.fold(
        WebRtcTransportListenInfos::new(first),
        WebRtcTransportListenInfos::insert,
    );
    let mut options = WebRtcTransportOptions::new(listen_infos);
    options.enable_udp = true;
    options.enable_tcp = true;
    options.prefer_udp = true;
    options.initial_available_outgoing_bitrate = INITIAL_OUTGOING_BITRATE;
    options
}

/// Where a plain transport listens.
pub(crate) fn local_tuple(transport: &PlainTransport) -> (String, u16) {
    match transport.tuple() {
        TransportTuple::WithRemote {
            local_address,
            local_port,
            ..
        }
        | TransportTuple::LocalOnly {
            local_address,
            local_port,
            ..
        } => (local_address, local_port),
    }
}

pub(crate) fn wire_kind(kind: MediaKind) -> WireKind {
    match kind {
        MediaKind::Audio => WireKind::Audio,
        MediaKind::Video => WireKind::Video,
    }
}

pub(crate) fn media_kind(kind: WireKind) -> MediaKind {
    match kind {
        WireKind::Audio => MediaKind::Audio,
        WireKind::Video => MediaKind::Video,
    }
}

/// The parameters of the H.264 the router offers and game capture sends: constrained baseline,
/// non-interleaved packetization.
pub(crate) fn h264_parameters() -> RtpCodecParametersParameters {
    let mut parameters = RtpCodecParametersParameters::default();
    parameters.insert("packetization-mode", 1u32);
    parameters.insert("profile-level-id", "42e01f");
    parameters.insert("level-asymmetry-allowed", 1u32);
    parameters
}

/// The codecs every router offers: Opus for voice, VP8 for what browsers share, and H.264 for
/// game capture.
pub(crate) fn media_codecs() -> Vec<RtpCodecCapability> {
    vec![
        RtpCodecCapability::Audio {
            mime_type: MimeTypeAudio::Opus,
            preferred_payload_type: None,
            clock_rate: NonZeroU32::new(48_000).expect("non-zero"),
            channels: NonZeroU8::new(2).expect("non-zero"),
            parameters: RtpCodecParametersParameters::from([
                ("useinbandfec", 1_u32.into()),
                ("usedtx", 1_u32.into()),
            ]),
            rtcp_feedback: vec![RtcpFeedback::TransportCc],
        },
        // Browsers send their camera and screen video as VP8; see H.264 below for the order.
        RtpCodecCapability::Video {
            mime_type: MimeTypeVideo::Vp8,
            preferred_payload_type: None,
            clock_rate: NonZeroU32::new(90_000).expect("non-zero"),
            parameters: RtpCodecParametersParameters::default(),
            rtcp_feedback: vec![
                RtcpFeedback::Nack,
                RtcpFeedback::NackPli,
                RtcpFeedback::CcmFir,
                RtcpFeedback::GoogRemb,
                RtcpFeedback::TransportCc,
            ],
        },
        // H.264 constrained baseline is what the desktop shell's game capture sends; every
        // browser decodes it, Firefox through OpenH264. The level is ignored on matching. It
        // comes after VP8 because a browser producing video takes the router's first codec it
        // can send, and Chromium's H.264 encoder is the software OpenH264, which drops frames
        // on screen content where its VP8 encoder keeps up.
        RtpCodecCapability::Video {
            mime_type: MimeTypeVideo::H264,
            preferred_payload_type: None,
            clock_rate: NonZeroU32::new(90_000).expect("clock rate"),
            parameters: h264_parameters(),
            rtcp_feedback: vec![
                RtcpFeedback::Nack,
                RtcpFeedback::NackPli,
                RtcpFeedback::CcmFir,
                RtcpFeedback::GoogRemb,
                RtcpFeedback::TransportCc,
            ],
        },
    ]
}
