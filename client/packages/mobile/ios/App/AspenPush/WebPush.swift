import CryptoKit
import Foundation

/// Decrypting a Web Push message (RFC 8291, `aes128gcm` of RFC 8188) with the subscription's
/// keys, as the deployment encrypted it (`server/src/app/push/webpush.rs`) and as Android's
/// `WebPush.java` reads it. The message is one record: a 16-byte salt, the record size, the
/// sender's public key, and the ciphertext with its tag.
enum WebPush {
    enum Failure: Error {
        case tooShort
        case badKey
        case badRecord
    }

    /// The plaintext of `message`, for a subscription whose private scalar is `privateKey` (the
    /// `d` of its JWK), whose public key is `publicKey` (an uncompressed P-256 point), and
    /// whose authentication secret is `auth` (16 bytes).
    static func decrypt(message: Data, privateKey: Data, publicKey: Data, auth: Data) throws -> Data {
        guard message.count >= 21 else {
            throw Failure.tooShort
        }
        let salt = message.prefix(16)
        let keyLength = Int(message[message.startIndex + 20])
        guard message.count >= 21 + keyLength else {
            throw Failure.tooShort
        }
        let senderKey = message.subdata(in: (message.startIndex + 21)..<(message.startIndex + 21 + keyLength))
        let record = message.subdata(in: (message.startIndex + 21 + keyLength)..<message.endIndex)

        let ours: P256.KeyAgreement.PrivateKey
        let theirs: P256.KeyAgreement.PublicKey
        do {
            ours = try P256.KeyAgreement.PrivateKey(rawRepresentation: privateKey)
            theirs = try P256.KeyAgreement.PublicKey(x963Representation: senderKey)
        } catch {
            throw Failure.badKey
        }
        let secret = try ours.sharedSecretFromKeyAgreement(with: theirs)
        var info = Data("WebPush: info\0".utf8)
        info.append(publicKey)
        info.append(senderKey)
        let ikm = secret.hkdfDerivedSymmetricKey(
            using: SHA256.self,
            salt: auth,
            sharedInfo: info,
            outputByteCount: 32
        )
        let cek = HKDF<SHA256>.deriveKey(
            inputKeyMaterial: ikm,
            salt: salt,
            info: Data("Content-Encoding: aes128gcm\0".utf8),
            outputByteCount: 16
        )
        let nonce = HKDF<SHA256>.deriveKey(
            inputKeyMaterial: ikm,
            salt: salt,
            info: Data("Content-Encoding: nonce\0".utf8),
            outputByteCount: 12
        )
        guard record.count > 16 else {
            throw Failure.badRecord
        }
        let sealed: AES.GCM.SealedBox
        do {
            sealed = try AES.GCM.SealedBox(
                nonce: AES.GCM.Nonce(data: nonce.withUnsafeBytes { Data($0) }),
                ciphertext: record.dropLast(16),
                tag: record.suffix(16)
            )
        } catch {
            throw Failure.badRecord
        }
        let padded: Data
        do {
            padded = try AES.GCM.open(sealed, using: cek)
        } catch {
            throw Failure.badRecord
        }
        // The record ends with its delimiter, 2 for the last record, then any zero padding.
        var end = padded.count
        while end > 0 && padded[padded.startIndex + end - 1] == 0 {
            end -= 1
        }
        guard end > 0, padded[padded.startIndex + end - 1] == 2 else {
            throw Failure.badRecord
        }
        return padded.prefix(end - 1)
    }
}

extension Data {
    /// The bytes `text` encodes in base64url, with or without padding.
    init?(base64url text: String) {
        var base64 = text.replacingOccurrences(of: "-", with: "+").replacingOccurrences(of: "_", with: "/")
        while base64.count % 4 != 0 {
            base64.append("=")
        }
        self.init(base64Encoded: base64)
    }
}
