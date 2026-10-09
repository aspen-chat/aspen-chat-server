# File transfers

While in a call, a participant holding Transfer files may offer a file to everyone else in it. Each participant who accepts starts one transfer between the two of them, over a WebRTC data channel of their own, encrypted end to end. The bytes never touch mediasoup, storage, or the API server. Design notes: [file-transfers-design-notes.md](file-transfers-design-notes.md).

## Where it lives

| Part | Where |
| --- | --- |
| Offers and transfers of each call | `voice_server/src/rooms/transfers.rs` |
| Signalling frames | `voice_protocol::signal` |
| STUN and TURN | `voice_server/src/transfer.rs` (the `turn` crate) |
| Record for moderators | `app::file_transfer`, tables `file_offer` and `file_transfer` |
| Client protocol | `FileTransfers` (`client/packages/protocol/src/transfers.ts`), which `VoiceCall` feeds its frames |
| Client UI | `FilesPanel`, `FileDialogs`, and `TransferLinks` in the app |
| Mobile file saving | `AspenFilesPlugin` (`client/packages/app/src/api/filesBridge.ts`) |

## Permission

- Transfer files is `transferFiles`. The join token carries it as `transfer_files`, and the API server sends it again whenever it changes.
- Losing it withdraws the participant's standing offers and ends the transfers they are sending, with reason `notPermitted` told to both sides (see [Voice](voice/index.md)).
- The deployment setting `file_transfers` (see [Administration](administration/index.md)) off turns the whole feature off: no join token then grants Transfer files.
- Turning `file_transfers` off or on rechecks every call (`Recheck::Everyone`), so the participants' grants follow at once.

## Offers

- An offer lasts as long as its sender chooses: sixty seconds by default, from `MIN_VALID_SECONDS` to `MAX_VALID_SECONDS` (an hour).
- Any number of people may accept one.
- The sender's client answers acceptances by itself, so an offer works while its sender is away.

## Routes

The sender chooses when offering whether receivers may connect directly. Each receiver must choose how their transfer travels among the ways the sender and the server allow, even when there is only one.

| Mode | Route |
| --- | --- |
| `directPreferred` | Hole-punched, which could expose each side's address to the other. Falls back to the relay when the server relays |
| `relayOnly` | Through the voice server's TURN relay, so neither learns the other's address |

The dialogs say so in those words, and name the relay's limit.

## Lifetime of a transfer

- A transfer outlives its offer, so one under way finishes after the offer runs out.
- Either side may cancel one at any time. That ends it at once on both sides and at the relay.
- Nothing resumes.

## Signalling

The voice server keeps the offers and transfers of each call and passes each transfer's peer connection signalling between its two sides.

| Client to server | Server to client |
| --- | --- |
| `offerFile` | `fileOffered` |
| `withdrawFile` | `fileWithdrawn` |
| `acceptFile` | `transferStarting` |
| `transferSignal` | `transferSignal` |
| `endTransfer` | `transferEnded` |

Everyone in the call is told when a transfer starts and ends and between whom (`transferLinkChanged`, and `links` in `ready`), never what it carries.

## STUN and TURN

The network side is `voice_server/src/transfer.rs`.

### STUN

One UDP port (`[transfer] port`, 3478) answers STUN, so no third party learns who is connecting.

### TURN credentials

- A credential names one side of one transfer, by the offer's `record` (the id the server made for it, never the client's).
- It works only while that transfer is live.
- It works only from `ADDRESSES_PER_CREDENTIAL` (two) client addresses. So one side holds at most two allocations, and no credential can take the relay's ports.
- Its allocations are deleted the moment the transfer ends.

### Relay

- The relay forwards only between its own live allocations (`LivePorts`), so never to a port of its range on the machine that no allocation holds. Every relayed transfer is relayed at both ends.
- Its ports may not overlap the media range or the STUN port. The server refuses to start otherwise.
- It is looped back inside the server, so it needs no hairpin NAT.

### Relay bandwidth

Every relayed transfer on the server shares `[transfer] relay_mbps` (50; 0 turns relaying off).

1. A pacer sends from one queue at that rate.
2. Past half full, it drops the odd datagram with rising probability.
3. It drops whatever overflows `QUEUE_SECONDS` of the queue.

## Client

- The client draws each transfer as a dashed line in the accent colour, marching from sender to receiver between their tiles, turning only at right angles (`TransferLinks`).
- Each transfer's row shows the mode the receiver preferred and the route actually in use, read from the peer connection's selected candidate pair (`routeOf`).

### Saving the file

Where the receiver can choose a place first, accepting opens a picker and the file is written there as it arrives (a `FileSink`). The file is closed only when every byte is in, and discarded when the transfer ends otherwise.

| Platform | How |
| --- | --- |
| Browser with the File System Access API (Chromium, so the desktop app too) | Picker, written as it arrives |
| Android | `AspenFilesPlugin`: the system create-document picker names the file |
| iOS | `AspenFilesPlugin`: iOS has no picker for a file yet to be written, so the user chooses a folder and the file is made there under the name it was sent with |
| Elsewhere (Firefox and Safari) | Held until it has all arrived, then saved |

## The record

The deployment keeps a record of every offer and transfer for its moderators (`app::file_transfer`, tables `file_offer` and `file_transfer`).

- It holds who offered what, by name and size, who received it, in which mode, and how it ended. Never the file.
- The voice server reports each offer (`VoiceReport::FileOffered`) and each transfer's start and end into the report stream (see [Voice](voice/index.md)).
- It names the offer by an id of its own (`record`, UUIDv7) rather than the id the client chose, so no client can make its offer collide with another in the record.
- A transfer's start is recorded once however many times its report arrives.
- A snapshot carries only calls, so a transfer report that is lost is missing from the record for good.

### Reading it

It tells who sent what to whom in private calls, so only holders of Moderate any community read it:

- `GET /admin/file-transfers`: newest offer first, paged by `before` and `limit`, one person's by `filter[user]` (as sender or receiver);
- the dashboard's File transfers section.
