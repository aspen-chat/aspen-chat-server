//! A call's media path end to end, against a real mediasoup worker: a client written in pure
//! Rust (webrtc-rs's STUN, DTLS and SRTP, none of which use OpenSSL) opens a WebRTC transport
//! made as a participant's is, completes ICE and the DTLS handshake, checks the server's
//! certificate against the fingerprint it announced, sends SRTP to a producer and decrypts what
//! a consumer of it sends back. The worker does its side with the OpenSSL it links, the
//! system's (`mediasoup-sys`'s `system-openssl`), so this fails if that OpenSSL cannot
//! handshake, export keys, or encrypt as libsrtp needs.

use crate::media::{media_codecs, webrtc_transport_options};
use async_trait::async_trait;
use dtls::config::Config as DtlsConfig;
use dtls::conn::DTLSConn;
use dtls::crypto::Certificate;
use dtls::extension::extension_use_srtp::SrtpProtectionProfile;
use mediasoup::prelude::*;
use mediasoup::types::data_structures::{DtlsFingerprint, DtlsRole, DtlsState};
use sha2::{Digest, Sha256};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::num::{NonZeroU8, NonZeroU32};
use std::sync::Arc;
use std::time::Duration;
use stun::attributes::{ATTR_ICE_CONTROLLING, ATTR_PRIORITY, ATTR_USE_CANDIDATE, ATTR_USERNAME};
use stun::fingerprint::FINGERPRINT;
use stun::integrity::MessageIntegrity;
use stun::message::{BINDING_REQUEST, BINDING_SUCCESS, Message, Setter};
use stun::textattrs::TextAttribute;
use tokio::net::UdpSocket;
use tokio::sync::{Mutex, mpsc, watch};
use tokio::time::timeout;
use webrtc_srtp::context::Context as SrtpContext;
use webrtc_srtp::protection_profile::ProtectionProfile;
use webrtc_util::Unmarshal;

/// How long each step (ICE, the handshake, media both ways) is given.
const STEP: Duration = Duration::from_secs(10);
/// The packets the client sends, 20 ms apart, as Opus does.
const PACKETS: u16 = 50;
const SSRC: u32 = 0x5eed_0001;
const OPUS_PAYLOAD_TYPE: u8 = 100;

/// Every profile the worker offers, newest first. Each takes a handshake of its own, and the
/// GCM ones need libsrtp's OpenSSL backend.
#[tokio::test]
async fn media_flows_over_dtls_srtp_with_every_profile() {
    // The client's DTLS takes rustls's process-wide provider, and the workspace enables more
    // than one; the client is given ring, which it uses for the rest of its crypto.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let worker = WorkerManager::new()
        .create_worker(WorkerSettings::default())
        .await
        .expect("worker starts");
    let router = worker
        .create_router(RouterOptions::new(media_codecs()))
        .await
        .expect("router");
    for (offered, profile) in [
        (
            SrtpProtectionProfile::Srtp_Aead_Aes_256_Gcm,
            ProtectionProfile::AeadAes256Gcm,
        ),
        (
            SrtpProtectionProfile::Srtp_Aead_Aes_128_Gcm,
            ProtectionProfile::AeadAes128Gcm,
        ),
        (
            SrtpProtectionProfile::Srtp_Aes128_Cm_Hmac_Sha1_80,
            ProtectionProfile::Aes128CmHmacSha1_80,
        ),
        (
            SrtpProtectionProfile::Srtp_Aes128_Cm_Hmac_Sha1_32,
            ProtectionProfile::Aes128CmHmacSha1_32,
        ),
    ] {
        round_trip(&router, offered, profile).await;
    }
}

/// The worker's OpenSSL is the system's shared library, not a copy built into it: only
/// mediasoup's worker links OpenSSL in this crate, so finding it mapped means the worker did.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn the_worker_uses_the_shared_system_openssl() {
    let _worker = WorkerManager::new()
        .create_worker(WorkerSettings::default())
        .await
        .expect("worker starts");
    let maps = std::fs::read_to_string("/proc/self/maps").expect("maps");
    for library in ["libssl.so", "libcrypto.so"] {
        assert!(
            maps.lines().any(|line| line.contains(library)),
            "{library} is not mapped; the worker has a copy of OpenSSL built in"
        );
    }
}

/// One participant's transport, negotiated to `offered`: ICE, DTLS, then `PACKETS` sent to a
/// producer and the same ones received back from a consumer of it.
async fn round_trip(router: &Router, offered: SrtpProtectionProfile, profile: ProtectionProfile) {
    let transport = router
        .create_webrtc_transport(webrtc_transport_options(
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            None,
        ))
        .await
        .expect("transport");
    // The worker reports its DTLS state in a notification, which may arrive after the client
    // has finished its side of the handshake.
    let (dtls_state_tx, mut dtls_state) = watch::channel(transport.dtls_state());
    let _dtls_state_handler = transport.on_dtls_state_change(move |state| {
        let _ = dtls_state_tx.send(state);
    });
    let candidate = transport
        .ice_candidates()
        .iter()
        .find(|candidate| candidate.protocol == Protocol::Udp)
        .expect("a UDP candidate")
        .clone();
    let server: SocketAddr = format!("{}:{}", candidate.address, candidate.port)
        .parse()
        .expect("candidate address");
    let ice = transport.ice_parameters().clone();
    let server_fingerprint = transport
        .dtls_parameters()
        .fingerprints
        .iter()
        .find_map(|fingerprint| match fingerprint {
            DtlsFingerprint::Sha256 { value } => Some(*value),
            _ => None,
        })
        .expect("a SHA-256 fingerprint");

    let certificate = Certificate::generate_self_signed(vec!["aspen-smoke".to_string()])
        .expect("client certificate");
    let client_fingerprint: [u8; 32] = Sha256::digest(&certificate.certificate[0]).into();
    transport
        .connect(WebRtcTransportRemoteParameters {
            dtls_parameters: DtlsParameters {
                role: DtlsRole::Client,
                fingerprints: vec![DtlsFingerprint::Sha256 {
                    value: client_fingerprint,
                }],
            },
        })
        .await
        .expect("transport connects");

    let socket = Arc::new(
        UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("client socket"),
    );
    let (stun_tx, mut stun_rx) = mpsc::unbounded_channel();
    let (dtls_tx, dtls_rx) = mpsc::unbounded_channel();
    let (rtp_tx, mut rtp_rx) = mpsc::unbounded_channel();
    let reader = tokio::spawn(demultiplex(Arc::clone(&socket), stun_tx, dtls_tx, rtp_tx));

    // ICE: mediasoup is ICE Lite, so the client is controlling and nominates the only pair.
    let mut request = Message::new();
    request
        .build(&[
            Box::new(BINDING_REQUEST),
            Box::new(stun::agent::TransactionId::new()),
            Box::new(TextAttribute::new(
                ATTR_USERNAME,
                format!("{}:smoke", ice.username_fragment),
            )),
        ])
        .expect("binding request");
    request.add(ATTR_PRIORITY, &0x7e00_00ffu32.to_be_bytes());
    request.add(ATTR_ICE_CONTROLLING, &rand::random::<u64>().to_be_bytes());
    request.add(ATTR_USE_CANDIDATE, &[]);
    MessageIntegrity::new_short_term_integrity(ice.password.clone())
        .add_to(&mut request)
        .expect("integrity");
    FINGERPRINT.add_to(&mut request).expect("fingerprint");
    socket
        .send_to(&request.raw, server)
        .await
        .expect("binding request sent");
    let response = timeout(STEP, stun_rx.recv())
        .await
        .expect("a binding response in time")
        .expect("socket open");
    let mut message = Message::new();
    message.unmarshal_binary(&response).expect("STUN response");
    assert_eq!(message.typ, BINDING_SUCCESS, "ICE was refused");
    assert_eq!(message.transaction_id, request.transaction_id);

    // DTLS, as the client, accepting only the certificate the transport announced.
    let config = DtlsConfig {
        certificates: vec![certificate],
        srtp_protection_profiles: vec![offered],
        insecure_skip_verify: true,
        verify_peer_certificate: Some(Arc::new(move |certificates, _| {
            let presented: [u8; 32] =
                Sha256::digest(certificates.first().ok_or(dtls::Error::ErrNoCertificates)?).into();
            if presented == server_fingerprint {
                Ok(())
            } else {
                Err(dtls::Error::Other(
                    "the server's certificate is not the one it announced".to_string(),
                ))
            }
        })),
        ..DtlsConfig::default()
    };
    let conn = Arc::new(Demultiplexed {
        socket: Arc::clone(&socket),
        server,
        dtls: Mutex::new(dtls_rx),
    });
    let dtls = timeout(STEP, DTLSConn::new(conn, config, true, None))
        .await
        .unwrap_or_else(|_| panic!("{offered:?}: the handshake took too long"))
        .unwrap_or_else(|e| panic!("{offered:?}: the handshake failed: {e}"));
    assert_eq!(dtls.selected_srtpprotection_profile(), offered);
    timeout(
        STEP,
        dtls_state.wait_for(|state| *state == DtlsState::Connected),
    )
    .await
    .unwrap_or_else(|_| panic!("{offered:?}: the worker never reported the handshake done"))
    .expect("transport open");

    let mut keys = webrtc_srtp::config::Config {
        profile,
        ..Default::default()
    };
    keys.extract_session_keys_from_dtls(dtls.connection_state().await, true)
        .await
        .expect("SRTP keys");
    let mut outgoing = SrtpContext::new(
        &keys.keys.local_master_key,
        &keys.keys.local_master_salt,
        profile,
        None,
        None,
    )
    .expect("outgoing SRTP");
    let mut incoming = SrtpContext::new(
        &keys.keys.remote_master_key,
        &keys.keys.remote_master_salt,
        profile,
        None,
        None,
    )
    .expect("incoming SRTP");

    let producer = transport
        .produce(ProducerOptions::new(MediaKind::Audio, opus_parameters()))
        .await
        .expect("producer");
    let capabilities: RtpCapabilities = serde_json::from_value(
        serde_json::to_value(router.rtp_capabilities()).expect("capabilities serialize"),
    )
    .expect("capabilities as a receiver states them");
    let consumer = transport
        .consume(ConsumerOptions::new(producer.id(), capabilities))
        .await
        .expect("consumer");
    let consumer_ssrc = consumer.rtp_parameters().encodings[0]
        .ssrc
        .expect("consumer SSRC");

    let mut ticker = tokio::time::interval(Duration::from_millis(20));
    for sequence in 0..PACKETS {
        ticker.tick().await;
        let packet = outgoing
            .encrypt_rtp(&opus_packet(sequence))
            .expect("encrypts");
        socket.send_to(&packet, server).await.expect("RTP sent");
    }

    // What comes back was decrypted by the worker, forwarded, and encrypted again.
    let mut received = 0;
    let deadline = tokio::time::Instant::now() + STEP;
    while received < PACKETS {
        let Ok(Some(packet)) = tokio::time::timeout_at(deadline, rtp_rx.recv()).await else {
            break;
        };
        let plain = incoming
            .decrypt_rtp(&packet)
            .unwrap_or_else(|e| panic!("{offered:?}: the worker's SRTP did not decrypt: {e}"));
        let parsed = rtp::packet::Packet::unmarshal(&mut &plain[..]).expect("RTP");
        if parsed.header.ssrc != consumer_ssrc {
            continue;
        }
        assert!(
            parsed.payload.starts_with(b"aspen-smoke-"),
            "{offered:?}: unexpected payload"
        );
        received += 1;
    }
    // Some may be lost even on loopback; most must arrive.
    assert!(
        received >= PACKETS * 9 / 10,
        "{offered:?}: {received} of {PACKETS} packets came back"
    );
    let stats = producer.get_stats().await.expect("producer stats");
    let packets: u64 = stats.iter().map(|stat| stat.packet_count).sum();
    assert!(
        packets >= u64::from(PACKETS) * 9 / 10,
        "{offered:?}: the producer counted {packets} of {PACKETS} packets"
    );

    reader.abort();
    let _ = dtls.close().await;
}

/// Opus as a client produces it on the payload type the router assigns.
fn opus_parameters() -> RtpParameters {
    RtpParameters {
        mid: None,
        msid: None,
        codecs: vec![RtpCodecParameters::Audio {
            mime_type: MimeTypeAudio::Opus,
            payload_type: OPUS_PAYLOAD_TYPE,
            clock_rate: NonZeroU32::new(48_000).expect("clock rate"),
            channels: NonZeroU8::new(2).expect("channels"),
            parameters: RtpCodecParametersParameters::default(),
            rtcp_feedback: vec![],
        }],
        header_extensions: vec![],
        encodings: vec![RtpEncodingParameters {
            ssrc: Some(SSRC),
            ..RtpEncodingParameters::default()
        }],
        rtcp: RtcpParameters {
            cname: Some("aspen-smoke".to_string()),
            reduced_size: true,
        },
    }
}

/// An RTP packet of one 20 ms Opus frame, its payload marked so it can be told apart.
fn opus_packet(sequence: u16) -> Vec<u8> {
    let timestamp = u32::from(sequence) * 960;
    let mut packet = vec![0x80, OPUS_PAYLOAD_TYPE];
    packet.extend_from_slice(&sequence.to_be_bytes());
    packet.extend_from_slice(&timestamp.to_be_bytes());
    packet.extend_from_slice(&SSRC.to_be_bytes());
    packet.extend_from_slice(format!("aspen-smoke-{sequence:04}").as_bytes());
    packet.resize(packet.len() + 60, 0);
    packet
}

/// Sorts what the socket receives by RFC 7983's first byte: STUN, DTLS, and RTP (RTCP, which
/// shares RTP's range, is dropped).
async fn demultiplex(
    socket: Arc<UdpSocket>,
    stun: mpsc::UnboundedSender<Vec<u8>>,
    dtls: mpsc::UnboundedSender<Vec<u8>>,
    rtp: mpsc::UnboundedSender<Vec<u8>>,
) {
    let mut buf = vec![0; 2048];
    while let Ok((n, _)) = socket.recv_from(&mut buf).await {
        let packet = buf[..n].to_vec();
        let _ = match (packet.first(), packet.get(1)) {
            (Some(0..=3), _) => stun.send(packet),
            (Some(20..=63), _) => dtls.send(packet),
            (Some(128..=191), Some(second)) if !(64..=95).contains(&(second & 0x7f)) => {
                rtp.send(packet)
            }
            _ => Ok(()),
        };
    }
}

/// The DTLS records of the client's socket, as the connection DTLS runs over.
struct Demultiplexed {
    socket: Arc<UdpSocket>,
    server: SocketAddr,
    dtls: Mutex<mpsc::UnboundedReceiver<Vec<u8>>>,
}

#[async_trait]
impl webrtc_util::Conn for Demultiplexed {
    async fn connect(&self, _addr: SocketAddr) -> webrtc_util::Result<()> {
        Ok(())
    }

    async fn recv(&self, buf: &mut [u8]) -> webrtc_util::Result<usize> {
        let record = self
            .dtls
            .lock()
            .await
            .recv()
            .await
            .ok_or(webrtc_util::Error::ErrClosedListener)?;
        let n = record.len().min(buf.len());
        buf[..n].copy_from_slice(&record[..n]);
        Ok(n)
    }

    async fn recv_from(&self, buf: &mut [u8]) -> webrtc_util::Result<(usize, SocketAddr)> {
        Ok((self.recv(buf).await?, self.server))
    }

    async fn send(&self, buf: &[u8]) -> webrtc_util::Result<usize> {
        Ok(self.socket.send_to(buf, self.server).await?)
    }

    async fn send_to(&self, buf: &[u8], _target: SocketAddr) -> webrtc_util::Result<usize> {
        self.send(buf).await
    }

    fn local_addr(&self) -> webrtc_util::Result<SocketAddr> {
        Ok(self.socket.local_addr()?)
    }

    fn remote_addr(&self) -> Option<SocketAddr> {
        Some(self.server)
    }

    async fn close(&self) -> webrtc_util::Result<()> {
        Ok(())
    }

    fn as_any(&self) -> &(dyn std::any::Any + Send + Sync) {
        self
    }
}
