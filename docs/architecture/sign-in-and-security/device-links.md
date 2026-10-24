# Device links

A device signs in another by a QR code, or by typing the user code shown beside it, through a device
link.

| Part            | Where                                                                 |
| --------------- | --------------------------------------------------------------------- |
| Links           | `app::device_link`, `/auth/device-links`, `/auth/device-link-entries` |
| The new sign-in | `login::issue_linked_session`                                         |
| Rate limits     | `rate_limits.toml`                                                    |

## Giver and receiver

- The **giver** is signed in. The **receiver** is not.
- Whichever starts the link shows its code. The other scans it, or types its user code.

| Kind      | Started by                                                                                                                    | Taken by                                                                                        |
| --------- | ----------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------- |
| `request` | A device that is not signed in: `POST /auth/device-links` without a session, naming itself and giving a PKCE `S256` challenge | A signed-in device: `POST …/{link}/scan`, or `POST /auth/device-link-entries`, with its session |
| `offer`   | A signed-in device                                                                                                            | A device that is not signed in, naming itself and giving the challenge                          |

A signed-in device that scans an offer is refused, before the code is used, since claiming would
replace its sign-in.

## Steps

1. The link's id is the QR code's secret. It is kept in Valkey under a digest, like the passkey
   ceremonies.
2. The link lasts a minute unscanned, or five minutes with a user code (`withCode`).
3. One scan or entry takes it: `SET NX` on a scan key. Of two devices taking it at once, one wins
   and the other gets `deviceLinkUsed`, which tells someone whose screen was photographed.
4. The giver confirms the receiver by the name it gave, with a tap: `PUT …/approval`. The giver of
   an offer asks `GET …/{link}` every couple of seconds to learn it was taken.
5. For a request taken by a typed code, the receiver confirms the giver's account (see
   [Confirming the giver](#confirming-the-giver)).
6. The receiver claims its sign-in with the verifier: `POST …/claim`, every couple of seconds. It
   returns the progress, then the sign-in, once.

Only the receiver can claim. A giver approving a stranger's request, or a stranger holding a
photographed code, gets no session.

## User codes

A link started with `withCode` also has a user code, as OAuth's device authorization grant has (RFC
8628, section 6.1). Apps ask for one on every link and show it beside the QR code. A link started
without it, as by an app that predates codes, has none and lasts its minute.

- The code is eight letters of RFC 8628's base-20 alphabet, `BCDFGHJKLMNPQRSTVWXZ`, about 34.5 bits.
  It is shown as `BCDF-GHJK`.
- It is kept in Valkey under its digest, pointing at the link's id, for the link's lifetime. It is
  reserved with `SET NX`, so no two live links share one. A new code is drawn on a collision.
- Typed input is read in compatibility form (NFKC, so full-width letters count), in any case, with
  hyphens and whitespace dropped. Input that cannot be a code at all is refused with `validation`
  and counts as nothing.
- Cancelling a link leaves its code reserved until it expires, matching nothing.

`POST /auth/device-link-entries` takes the link a code names, as a scan does, and answers its id and
what a scan answers. The side is the caller's: signed in, the code must be a request's; signed out,
an offer's.

### Guessing

A user code can be guessed, where the id cannot. So:

- Whatever the caller lacks (a recent verification, a device name, a challenge) is refused before
  the code is looked at, so the refusal says nothing about the code.
- A code that is unknown, expired, cancelled, or the other side's kind is `deviceLinkCodeUnknown`,
  alike. A guess learns nothing about other links.
- Each entry counts as a failure before its code is looked at (`Failures::take`), and is given back
  if it proves right (`Failures::forgive`). So entries sent at once cannot all pass while none has
  failed yet. A code whose link is already taken (`deviceLinkUsed`) is not given back.
- Failures count toward the address (`ip_failures`) for someone signed out, the account
  (`user_failures`) for someone signed in, and everyone (`global_failures`) either way.

## Confirming the giver

A giver who typed a request's code may have guessed it. Approving would then sign the stranger's
device in to the guesser's account. So, for a request taken by a typed code:

- The receiver's claim answers `confirmAccount` with the giver's account: id, username, display
  name, and icon.
- The receiver shows it and asks its person whether that is the account they mean to sign in to.
- Agreeing, the receiver claims again naming it (`account`). Declining, it cancels the link.
- The claim issues nothing until the giver has approved and the claim names the giver's account.

Once the giver approves, the link waits a minute for this, rather than the half minute a claim
otherwise has.

A request taken by its QR code asks for no such confirmation. The id is unguessable, so its giver
saw the receiver's screen. An offer's receiver typed or scanned the code itself, and the entry or
scan names the account it will be signed in to.

## Cancelling

`DELETE …/{link}` removes the link. A scan or approval under way at that moment finds it gone rather
than writing it back (`SET XX`), so a cancelled code stays cancelled.

## Device names

Nothing about the devices is looked up. A device's name is what it calls itself ("Firefox on
Linux"). No address or place is recorded.

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

Giving one, by starting an offer or taking a request, is a security change. It takes a sign-in
verified within `[auth] reverify_seconds`, checked before the code is used
(`reauthenticationRequired`).

**Why:** without it a stolen session could mint a sign-in of its own on another device, outliving
the stolen one.

## Rate limits

Starting a link, like starting a passkey ceremony, is limited per address. It is also limited,
generously, by everyone's starts together (`rate_limits.toml`), since each keeps state in Valkey
until it expires. The per-address limit is well under that one, so one address cannot spend
everyone's starts.

Typing a code is limited by its failures (see [Guessing](#guessing)). The built-in `global_failures`
allows 60 wrong codes in ten minutes, deployment-wide. While it is spent, typed codes are refused
for everyone until it refills, and QR codes still work.

## When access is given or taken away

1. **Who can observe a link?** Only whoever holds its id, which travels in the QR code's URL
   fragment and the request paths, or its user code, which only takes it. The giver's own reads are
   refused to anyone else with `deviceLinkExpired`, as for an unknown link.
2. **What decides it?** The giver's sign-in. The link records its `sign_in_id`. The claim issues
   nothing unless a live, unexpired refresh token of the giver, of an undeleted account, still has
   that id, and `issue_session`'s ban check passes. For a request taken by a typed code, the
   receiver's confirmation of the giver's account too.
3. **When access is lost**, between the tap and the claim, the claim is stopped by:
   - Signing out.
   - Being signed out by a password change or a first second factor.
   - Deletion.
   - A ban from the deployment.

   After the claim the new sign-in is an ordinary one, ended by all the same things.

4. **When it is gained:** the receiver learns of its sign-in from the claim's answer, and the giver
   that the link was taken by polling `GET …/{link}`.
5. **Does every path announce it?** Nothing is announced. A link has no event, and both sides poll
   it while it lasts.
6. **Is it published inside the transaction?** Nothing is published. `login::issue_linked_session`
   writes the new sign-in's refresh token and session in one transaction.

`scripts/check_permissions.py` checks the sign-out and password-change cases, and that a giver whose
verification is stale is asked to verify again, for links taken by scanning and by typing. It also
checks that a typed request signs nothing in until its receiver names the giver's account, and that
a wrong code is refused alike whatever it missed.
