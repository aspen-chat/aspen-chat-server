"""Device credentials for the scripts' requests made without a session.

A deployment asks every request without a session to present one (`Aspen-Device`; see
docs/architecture/sign-in-and-security/device-credentials.md), earned by solving a puzzle: a
nonce for which SHA-256(SHA-256(challenge) || nonce), with the nonce as eight big-endian bytes,
begins with the challenge's number of zero bits. The scripts earn one per deployment and share
it among the accounts they make, as one device would; those that make many accounts turn the
limits off in the aspen.toml they write.
"""

import base64
import hashlib
import json
from typing import Callable

HEADER = "aspen-device"

# Asks `(method, path under /api/v1, body)` and answers `(status, text)`.
Post = Callable[[str, str, dict], tuple[int, str]]


def zero_bits(digest: bytes) -> int:
    bits = 0
    for byte in digest:
        if byte:
            return bits + 8 - byte.bit_length()
        bits += 8
    return bits


def solve(challenge: str, difficulty: int) -> int:
    """The first nonce solving `challenge`, as base64url without padding."""
    sealed = base64.urlsafe_b64decode(challenge + "=" * (-len(challenge) % 4))
    prefix = hashlib.sha256(hashlib.sha256(sealed).digest())
    nonce = 0
    while True:
        hashed = prefix.copy()
        hashed.update(nonce.to_bytes(8, "big"))
        if zero_bits(hashed.digest()) >= difficulty:
            return nonce
        nonce += 1


def earn(post: Post) -> str:
    """Asks for a challenge, solves it, and redeems the solution."""
    status, text = post("POST", "/auth/device-challenges", {})
    if status != 200:
        raise RuntimeError(f"POST /auth/device-challenges: {status} {text[:300]}")
    challenge = json.loads(text)
    nonce = solve(challenge["challenge"], challenge["difficulty"])
    status, text = post("POST", "/auth/device-credentials", {"challenge": challenge["challenge"], "nonce": nonce})
    if status != 200:
        raise RuntimeError(f"POST /auth/device-credentials: {status} {text[:300]}")
    return json.loads(text)["credential"]


class Devices:
    """One credential per deployment, earned when first needed."""

    def __init__(self) -> None:
        self._earned: dict[str, str] = {}

    def credential(self, deployment: str, post: Post) -> str:
        if deployment not in self._earned:
            self._earned[deployment] = earn(post)
        return self._earned[deployment]
