// RFC 8291's own example (Appendix A), which the server's `webpush.rs` encrypts to and the
// extension's `WebPush.swift` must read back: run by `scripts/check_webpush.sh`.
import Foundation

let message = Data(base64url: "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPTpK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_Qulcy4a-fN")!
let privateKey = Data(base64url: "q1dXpw3UpT5VOmu_cf_v6ih07Aems3njxI-JWgLcM94")!
let publicKey = Data(base64url: "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4")!
let auth = Data(base64url: "BTBZMqHH6r4Tts7J_aSIgg")!
do {
    let plain = try WebPush.decrypt(message: message, privateKey: privateKey, publicKey: publicKey, auth: auth)
    let text = String(decoding: plain, as: UTF8.self)
    if text == "When I grow up, I want to be a watermelon" {
        print("ok: \(text)")
        exit(0)
    }
    print("wrong plaintext: \(text)")
    exit(1)
} catch {
    print("failed: \(error)")
    exit(1)
}
