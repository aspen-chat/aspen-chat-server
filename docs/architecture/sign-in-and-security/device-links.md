# Device links

A device signs in another by a QR code through a device link.

| Part | Where |
| --- | --- |
| Links | `app::device_link`, `/auth/device-links` |
| The new sign-in | `login::issue_linked_session` |
| Rate limits | `rate_limits.toml` |

## Giver and receiver

- The **giver** is signed in. The **receiver** is not.
- A computer (the desktop app or a browser) always shows the code. A phone always scans it.

| Kind | Started by | Scanned by |
| --- | --- | --- |
| `request` | A computer that is not signed in: `POST /auth/device-links` without a session, naming itself and giving a PKCE `S256` challenge | A signed-in phone: `POST …/{link}/scan` with its session |
| `offer` | A signed-in computer | A phone that is not signed in, naming itself and giving the challenge |

A signed-in phone that scans an offer is refused, before the code is used, since claiming would replace its sign-in.

## Steps

1. The link's id is the code's secret. It is kept in Valkey under a digest, like the passkey ceremonies.
2. The link lasts a minute unscanned.
3. One scan takes it: `SET NX` on a scan key. Of two devices scanning at once, one wins and the other gets `deviceLinkUsed`, which tells someone whose screen was photographed.
4. The giver confirms the receiver by the name it gave, with a tap: `PUT …/approval`. The giver of an offer asks `GET …/{link}` every couple of seconds to learn it was scanned.
5. The receiver claims its sign-in with the verifier: `POST …/claim`, every couple of seconds. It returns the progress, then the sign-in, once.

Only the receiver can claim. A giver approving a stranger's request, or a stranger holding a photographed code, gets no session.

## Cancelling

`DELETE …/{link}` removes the link. A scan or approval under way at that moment finds it gone rather than writing it back (`SET XX`), so a cancelled code stays cancelled.

## Device names

Nothing about the devices is looked up. A device's name is what it calls itself ("Firefox on Linux"). No address or place is recorded.

## The new sign-in

`login::issue_linked_session` records the giver's `method` and `verified_at`. So the new sign-in:

- Is no stronger than the giver's.
- Satisfies `require_two_factor` exactly when the giver's did.
- Makes no security change without verifying again once the giver's verification is stale.

## Who may give a sign-in

These cannot give one:

- Bots. Their sign-ins are their token's.
- Users of other deployments. Their sign-ins are their home's.
- A session still owing a second factor.

Giving one, by starting an offer or scanning a request, is a security change. It takes a sign-in verified within `[auth] reverify_seconds`, checked before the code is used (`reauthenticationRequired`).

**Why:** without it a stolen session could mint a sign-in of its own on another device, outliving the stolen one.

## Rate limits

Starting a link, like starting a passkey ceremony, is limited per address. It is also limited, generously, by everyone's starts together (`rate_limits.toml`), since each keeps state in Valkey until it expires.

## When access is given or taken away

1. **Who can observe a link?** Only whoever holds its id, which travels in the QR code's URL fragment and the request paths. The giver's own reads are refused to anyone else with `deviceLinkExpired`, as for an unknown link.
2. **What decides it?** The giver's sign-in. The link records its `sign_in_id`. The claim issues nothing unless a live, unexpired refresh token of the giver, of an undeleted account, still has that id, and `issue_session`'s ban check passes.
3. **When access is lost**, between the tap and the claim, the claim is stopped by:
   - Signing out.
   - Being signed out by a password change or a first second factor.
   - Deletion.
   - A ban from the deployment.

   After the claim the new sign-in is an ordinary one, ended by all the same things.
4. **When it is gained:** the receiver learns of its sign-in from the claim's answer, and the giver that the link was scanned by polling `GET …/{link}`.
5. **Does every path announce it?** Nothing is announced. A link has no event, and both sides poll it for its minute.
6. **Is it published inside the transaction?** Nothing is published. `login::issue_linked_session` writes the new sign-in's refresh token and session in one transaction.

`scripts/check_permissions.py` checks the sign-out and password-change cases, and that a giver whose verification is stale is asked to verify again.
