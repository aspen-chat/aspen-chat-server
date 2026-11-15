//! Telling a keyframe from the first packet of a frame, so a video consumer starts on one.

use str0m::format::Codec;

/// Whether `payload`, of `codec`, starts a keyframe. Anything but H.264 and VP8 never does,
/// which only audio is.
pub fn is_keyframe(codec: Codec, payload: &[u8]) -> bool {
    match codec {
        Codec::H264 => is_h264_keyframe(payload),
        Codec::Vp8 => is_vp8_keyframe(payload),
        _ => false,
    }
}

/// An IDR slice or a sequence parameter set, alone, aggregated (STAP-A), or the first fragment
/// of one (FU-A).
fn is_h264_keyframe(payload: &[u8]) -> bool {
    let Some(&first) = payload.first() else {
        return false;
    };
    let starts_picture = |nal: u8| matches!(nal & 0x1f, 5 | 7);
    match first & 0x1f {
        24 => {
            let mut at = 1;
            while at + 2 < payload.len() {
                let size = u16::from_be_bytes([payload[at], payload[at + 1]]) as usize;
                if starts_picture(payload[at + 2]) {
                    return true;
                }
                at += 2 + size;
            }
            false
        }
        28 => payload
            .get(1)
            .is_some_and(|&fu| fu & 0x80 != 0 && starts_picture(fu)),
        nal => starts_picture(nal),
    }
}

/// The start of partition 0 of a frame (S set, PID 0) whose payload header says keyframe (P
/// clear), read past the descriptor's optional fields.
fn is_vp8_keyframe(payload: &[u8]) -> bool {
    let Some(&descriptor) = payload.first() else {
        return false;
    };
    if descriptor & 0x10 == 0 || descriptor & 0x07 != 0 {
        return false;
    }
    let mut at = 1;
    if descriptor & 0x80 != 0 {
        let Some(&extension) = payload.get(at) else {
            return false;
        };
        at += 1;
        if extension & 0x80 != 0 {
            let two_bytes = payload.get(at).is_some_and(|&id| id & 0x80 != 0);
            at += if two_bytes { 2 } else { 1 };
        }
        if extension & 0x40 != 0 {
            at += 1;
        }
        if extension & 0x30 != 0 {
            at += 1;
        }
    }
    payload.get(at).is_some_and(|&header| header & 0x01 == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn h264_keyframes_are_found_alone_aggregated_and_fragmented() {
        assert!(is_keyframe(Codec::H264, &[0x67, 1, 2]));
        assert!(is_keyframe(Codec::H264, &[0x65, 1, 2]));
        assert!(!is_keyframe(Codec::H264, &[0x41, 1, 2]));
        // STAP-A holding a PPS then an SPS.
        assert!(is_keyframe(
            Codec::H264,
            &[0x78, 0, 2, 0x68, 0, 0, 2, 0x67, 0]
        ));
        assert!(!is_keyframe(Codec::H264, &[0x78, 0, 2, 0x68, 0]));
        // FU-A: the first fragment of an IDR, and a later one.
        assert!(is_keyframe(Codec::H264, &[0x7c, 0x85, 0]));
        assert!(!is_keyframe(Codec::H264, &[0x7c, 0x05, 0]));
    }

    #[test]
    fn vp8_keyframes_are_read_past_the_descriptor() {
        assert!(is_keyframe(Codec::Vp8, &[0x10, 0x00]));
        assert!(!is_keyframe(Codec::Vp8, &[0x10, 0x01]));
        // Not the start of a partition.
        assert!(!is_keyframe(Codec::Vp8, &[0x00, 0x00]));
        // X, I with a 15-bit picture id, then the payload header.
        assert!(is_keyframe(Codec::Vp8, &[0x90, 0x80, 0x81, 0x02, 0x00]));
    }

    #[test]
    fn audio_is_never_a_keyframe() {
        // Opus's first byte can look like an H.264 NAL header.
        assert!(!is_keyframe(Codec::Opus, &[0x67, 0x00]));
    }
}
