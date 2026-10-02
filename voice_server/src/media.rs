//! The mediasoup settings every router and plain transport shares: the codecs a router
//! offers, the H.264 parameters game capture sends, where a plain transport listens, and the
//! conversion between mediasoup's media kinds and the signalling protocol's.

use mediasoup::prelude::*;
use mediasoup::types::data_structures::TransportTuple;
use std::num::{NonZeroU8, NonZeroU32};
use voice_protocol::signal::MediaKind as WireKind;

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
