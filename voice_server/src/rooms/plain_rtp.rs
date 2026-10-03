//! Media a client sends or receives as SRTP itself rather than through a browser's WebRTC:
//! game capture's helper and the benchmark's simulated participants (`produceRtp`,
//! `consumeRtp`).

use super::{ReceiveTransport, Room, RoomError, Rooms};
use crate::media::{h264_parameters, local_tuple};
use mediasoup::prelude::*;
use mediasoup::types::srtp_parameters::SrtpParameters;
use std::num::{NonZeroU8, NonZeroU32};
use uuid::Uuid;
use voice_protocol::signal::{MediaSource, ServerMessage};

impl Rooms {
    /// A plain transport for a client that sends or receives SRTP itself: RTP and RTCP on one
    /// port, the client's address learned from its first packet, and one key both ways.
    async fn plain_transport(
        &self,
        room: &Room,
    ) -> Result<(PlainTransport, SrtpParameters), RoomError> {
        let listen = ListenInfo {
            protocol: Protocol::Udp,
            ip: self.rtc_ip,
            announced_address: self.announced_address.clone(),
            expose_internal_ip: false,
            port: None,
            port_range: None,
            flags: None,
            send_buffer_size: None,
            recv_buffer_size: None,
        };
        let mut options = PlainTransportOptions::new(listen);
        options.rtcp_mux = true;
        options.comedia = true;
        options.enable_srtp = true;
        options.srtp_crypto_suite = SrtpCryptoSuite::AesCm128HmacSha180;
        let transport = room.router.create_plain_transport(options).await?;
        let srtp = transport
            .srtp_parameters()
            .ok_or(RoomError::NoSrtpParameters)?;
        transport
            .connect(PlainTransportRemoteParameters {
                ip: None,
                port: None,
                rtcp_port: None,
                srtp_parameters: Some(srtp.clone()),
            })
            .await?;
        Ok((transport, srtp))
    }

    /// Moves the participant's consumers onto a plain SRTP transport (`consumeRtp`). Consumers
    /// already made on a WebRTC transport are left where they are; the participant, not being a
    /// browser, has none.
    pub async fn consume_rtp(&self, channel: Uuid, user: Uuid) -> Result<(), RoomError> {
        let room = self.room(channel)?;
        {
            let participants = room.participants.lock().expect("room lock");
            let participant = participants.get(&user).ok_or(RoomError::NotInCall)?;
            if participant.recv_transport.is_some() {
                return Err(RoomError::BadParameters(
                    "the participant already has a receive transport".to_string(),
                ));
            }
        }
        let (transport, srtp) = self.plain_transport(&room).await?;
        let (local_address, local_port) = local_tuple(&transport);
        {
            let mut participants = room.participants.lock().expect("room lock");
            let participant = participants.get_mut(&user).ok_or(RoomError::NotInCall)?;
            participant.recv_transport = Some(ReceiveTransport::Plain(transport));
            participant.send(ServerMessage::RtpConsuming {
                ip: local_address.to_string(),
                port: local_port,
                srtp_crypto_suite: "AES_CM_128_HMAC_SHA1_80".to_string(),
                srtp_key_base64: srtp.key_base64.clone(),
            });
        }
        self.ensure_consumers(&room, user).await;
        Ok(())
    }

    /// Makes a producer fed by SRTP the client sends itself, on a plain transport that learns
    /// the sender's address from its first packet, and tells the client where to send. The
    /// client consumes the producer too, as its own preview.
    pub async fn produce_rtp(
        &self,
        channel: Uuid,
        user: Uuid,
        source: MediaSource,
    ) -> Result<(), RoomError> {
        let room = self.room(channel)?;
        {
            let participants = room.participants.lock().expect("room lock");
            participants
                .get(&user)
                .ok_or(RoomError::NotInCall)?
                .ensure_source_free(source)?;
        }
        let (transport, srtp) = self.plain_transport(&room).await?;
        let ssrc = (Uuid::now_v7().as_u128() as u32) | 1;
        // Video is H.264 (the helper's x264), audio Opus (the helper's ffmpeg encoder, or a
        // simulated participant's); the payload types are the producer's own and need only be
        // distinct from each other.
        let (kind, payload_type, codec) = match source {
            MediaSource::ScreenAudio | MediaSource::Microphone => (
                MediaKind::Audio,
                100,
                RtpCodecParameters::Audio {
                    mime_type: MimeTypeAudio::Opus,
                    payload_type: 100,
                    clock_rate: NonZeroU32::new(48_000).expect("clock rate"),
                    channels: NonZeroU8::new(2).expect("channels"),
                    // Stereo: consumers take their codec parameters from the producer, and
                    // a browser decodes Opus as mono unless they carry `sprop-stereo`.
                    parameters: RtpCodecParametersParameters::from([(
                        "sprop-stereo",
                        1_u32.into(),
                    )]),
                    rtcp_feedback: vec![],
                },
            ),
            _ => (
                MediaKind::Video,
                96,
                RtpCodecParameters::Video {
                    mime_type: MimeTypeVideo::H264,
                    payload_type: 96,
                    clock_rate: NonZeroU32::new(90_000).expect("clock rate"),
                    parameters: h264_parameters(),
                    rtcp_feedback: vec![
                        RtcpFeedback::Nack,
                        RtcpFeedback::NackPli,
                        RtcpFeedback::CcmFir,
                        RtcpFeedback::GoogRemb,
                    ],
                },
            ),
        };
        let rtp_parameters = RtpParameters {
            mid: None,
            msid: None,
            codecs: vec![codec],
            header_extensions: if kind == MediaKind::Video {
                vec![RtpHeaderExtensionParameters {
                    uri: RtpHeaderExtensionUri::AbsSendTime,
                    id: 4,
                    encrypt: false,
                }]
            } else {
                vec![]
            },
            encodings: vec![RtpEncodingParameters {
                ssrc: Some(ssrc),
                ..RtpEncodingParameters::default()
            }],
            rtcp: RtcpParameters {
                cname: Some(format!("aspen-{user}")),
                reduced_size: true,
            },
        };
        let producer = transport
            .produce(ProducerOptions::new(kind, rtp_parameters))
            .await?;
        Self::observe_audio(&room, &producer, user).await;
        let (local_address, local_port) = local_tuple(&transport);
        let producer_id = producer.id();
        let state = {
            let mut participants = room.participants.lock().expect("room lock");
            let participant = participants.get_mut(&user).ok_or(RoomError::NotInCall)?;
            participant
                .producers
                .insert(producer_id, (producer, source));
            // Only video is previewed back to the sender; their own audio would be an echo.
            if kind == MediaKind::Video {
                participant.own_preview.insert(producer_id);
            }
            participant.rtp_transports.insert(producer_id, transport);
            participant.send(ServerMessage::RtpProduced {
                producer_id: producer_id.to_string(),
                source,
                ip: local_address.to_string(),
                port: local_port,
                ssrc,
                payload_type,
                srtp_crypto_suite: "AES_CM_128_HMAC_SHA1_80".to_string(),
                srtp_key_base64: srtp.key_base64.clone(),
            });
            (source == MediaSource::Screen).then(|| Self::state_report(&room, participant))
        };
        if let Some(report) = state {
            self.reporter.report(report);
        }
        let everyone: Vec<Uuid> = room
            .participants
            .lock()
            .expect("room lock")
            .keys()
            .copied()
            .collect();
        for member in everyone {
            self.ensure_consumers(&room, member).await;
        }
        Ok(())
    }
}
