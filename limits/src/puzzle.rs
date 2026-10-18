//! The puzzle a device credential is earned with (`app::device_credential`): a nonce for which
//! `SHA-256(SHA-256(challenge) || nonce)`, with the nonce as eight big-endian bytes, begins with
//! a number of zero bits. Here so the API server, which checks solutions, and the benchmark,
//! which solves them, share one definition; the client's solver passes the same vectors
//! (`spec/device_puzzle_vectors.json`).

use sha2::{Digest, Sha256};

/// How many leading zero bits `nonce` earns against `challenge`, the challenge's bytes as
/// decoded from its base64url.
pub fn work(challenge: &[u8], nonce: u64) -> u32 {
    zero_bits(
        &Sha256::new().chain_update(Sha256::digest(challenge)),
        nonce,
    )
}

/// The first nonce that earns `bits` zero bits against `challenge`: about `2^bits` hashes.
pub fn solve(challenge: &[u8], bits: u8) -> u64 {
    let prefix = Sha256::new().chain_update(Sha256::digest(challenge));
    (0..)
        .find(|nonce| zero_bits(&prefix, *nonce) >= u32::from(bits))
        .expect("some nonce solves the puzzle")
}

fn zero_bits(prefix: &Sha256, nonce: u64) -> u32 {
    let digest = prefix.clone().chain_update(nonce.to_be_bytes()).finalize();
    let mut bits = 0;
    for byte in digest {
        bits += byte.leading_zeros();
        if byte != 0 {
            break;
        }
    }
    bits
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Base64url without padding, as the vectors write challenges.
    fn decode(text: &str) -> Vec<u8> {
        const ALPHABET: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let mut bytes = Vec::new();
        let (mut buffer, mut held) = (0u32, 0);
        for c in text.bytes() {
            let value = ALPHABET.iter().position(|a| *a == c).expect("base64url") as u32;
            buffer = (buffer << 6) | value;
            held += 6;
            if held >= 8 {
                held -= 8;
                bytes.push((buffer >> held) as u8);
            }
        }
        bytes
    }

    #[test]
    fn work_matches_the_shared_vectors() {
        let vectors: serde_json::Value =
            serde_json::from_str(include_str!("../../spec/device_puzzle_vectors.json")).unwrap();
        for vector in vectors["vectors"].as_array().unwrap() {
            let challenge = decode(vector["challenge"].as_str().unwrap());
            let nonce = vector["nonce"].as_u64().unwrap();
            assert_eq!(
                u64::from(work(&challenge, nonce)),
                vector["zeroBits"].as_u64().unwrap(),
                "{vector}"
            );
        }
    }

    #[test]
    fn a_solution_earns_its_bits_and_is_the_first() {
        let challenge = b"challenge";
        let nonce = solve(challenge, 10);
        assert!(work(challenge, nonce) >= 10);
        assert!((0..nonce).all(|earlier| work(challenge, earlier) < 10));
    }
}
