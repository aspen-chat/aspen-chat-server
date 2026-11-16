# File transfers

While in a call, a participant holding Transfer files may offer a file to everyone else in it. Each
participant who accepts starts one transfer between the two of them, over a WebRTC data channel of
their own, encrypted end to end. The bytes never touch the voice server's media threads, storage, or
the API server. Design notes: [file-transfers-design-notes.md](file-transfers-design-notes.md).

## Where it lives

| Part                              | Where                                                                                             |
| --------------------------------- | ------------------------------------------------------------------------------------------------- |
| Offers and transfers of each call | `voice_server/src/rooms/transfers.rs`                                                             |
| Signalling frames                 | `voice_protocol::signal`                                                                          |
| STUN and TURN                     | `voice_server/src/transfer.rs` (the `turn` crate)                                                 |
| Record for moderators             | `app::file_transfer`, tables `file_offer` and `file_transfer`                                     |
| Client protocol                   | `FileTransfers` (`client/packages/protocol/src/transfers.ts`), which `VoiceCall` feeds its frames |
| The mobile call engine's          | `voice_client::transfers`, which `call` feeds its frames, over `voice_client::files`              |
| Client UI                         | `FilesPanel`, `FileDialogs`, and `TransferLinks` in the app                                       |
| Mobile files                      | `AspenFilesPlugin` (`client/packages/app/src/api/filesBridge.ts`), `FileRegistry`, `CallFiles`    |

## Permission

- Transfer files is `transferFiles`. The join token carries it as `transfer_files`, and the API
  server sends it again whenever it changes.
- Losing it withdraws the participant's standing offers and ends the transfers they are sending,
  with reason `notPermitted` told to both sides (see [Voice](voice/index.md)).
- The deployment setting `file_transfers` (see [Administration](administration/index.md)) off turns
  the whole feature off: no join token then grants Transfer files.
- Turning `file_transfers` off or on rechecks every call (`Recheck::Everyone`), so the participants'
  grants follow at once.

## Offers

- An offer lasts as long as its sender chooses: thirty minutes by default, from `MIN_VALID_SECONDS`
  to `MAX_VALID_SECONDS` (an hour).
- Any number of people may accept one.
- The sender's client answers acceptances by itself, so an offer works while its sender is away.

## Routes

The sender chooses when offering whether receivers may connect directly. Each receiver must choose
how their transfer travels among the ways the sender and the server allow, even when there is only
one.

| Mode              | Route                                                                                                             |
| ----------------- | ----------------------------------------------------------------------------------------------------------------- |
| `directPreferred` | Hole-punched, which could expose each side's address to the other. Falls back to the relay when the server relays |
| `relayOnly`       | Through the voice server's TURN relay, so neither learns the other's address                                      |

The dialogs say so in those words, and name the relay's limit.

## Lifetime of a transfer

- A transfer outlives its offer, so one under way finishes after the offer runs out.
- Either side may cancel one at any time. That ends it at once on both sides and at the relay.
- Nothing resumes.

## Signalling

The voice server keeps the offers and transfers of each call and passes each transfer's peer
connection signalling between its two sides.

| Client to server | Server to client   |
| ---------------- | ------------------ |
| `offerFile`      | `fileOffered`      |
| `withdrawFile`   | `fileWithdrawn`    |
| `acceptFile`     | `transferStarting` |
| `transferSignal` | `transferSignal`   |
| `endTransfer`    | `transferEnded`    |

Everyone in the call is told when a transfer starts and ends and between whom
(`transferLinkChanged`, and `links` in `ready`), never what it carries.

## STUN and TURN

The network side is `voice_server/src/transfer.rs`.

### STUN

One UDP port (`[transfer] port`, 3478) answers STUN, so no third party learns who is connecting. It
is bound on each media address (see [Voice](voice/voice-server.md#ipv4-and-ipv6)), and a transfer's
ICE servers name every one, an IPv6 address in brackets; a client uses those it can reach.

### TURN credentials

- A credential names one side of one transfer, by the offer's `record` (the id the server made for
  it, never the client's).
- It works only while that transfer is live.
- It works only from `ADDRESSES_PER_CREDENTIAL` (four) client addresses: a browser allocates from
  each interface it gathers on, once over each family. So one side holds at most four allocations,
  and no credential can take the relay's ports.
- Its allocations are deleted the moment the transfer ends.

### Relay

- The relay forwards only between its own live allocations (`LivePorts`), so never to a port of its
  range on the machine that no allocation holds. Every relayed transfer is relayed at both ends.
- Its ports may not overlap the media threads' (`[rtc] min_port` to `max_port`) or the STUN port.
  The server refuses to start otherwise.
- It is looped back inside the server, so it needs no hairpin NAT.
- Every allocation is announced at one address (`relay_address`): the IPv4 media address's when
  there is one, else the IPv6 one's, whichever family the client reached TURN over. A request for an
  IPv6 allocation is refused where that address is IPv4.

### Relay bandwidth

Every relayed transfer on the server shares `[transfer] relay_mbps` (50; 0 turns relaying off).

1. A pacer sends from one queue at that rate.
2. Past half full, it drops the odd datagram with rising probability.
3. It drops whatever overflows `QUEUE_SECONDS` of the queue.

## Client

- The client draws each transfer as a dashed line in the accent colour, marching from sender to
  receiver between their tiles, turning only at right angles (`TransferLinks`).
- Each transfer's row shows the mode the receiver preferred and the route actually in use, read from
  the peer connection's selected candidate pair (`routeOf`).

Two implementations run the client's side of a transfer on the same frames, so a phone and a browser
transfer files between them: `FileTransfers` in TypeScript for browsers and the desktop app, and
`voice_client::transfers` in Rust in the mobile apps' call engine (`voice_client/README.md`;
`client/docs/architecture/voice/native-calls.md` is the page's side). Both decide alike:

| Step             | Both                                                                                                                                                                                                                                                                                                                                    |
| ---------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Offering         | `offerFile` under an id the client chose, `validForSeconds` clamped to `MIN_VALID_SECONDS` and `MAX_VALID_SECONDS`; the offer shows once the server's `fileOffered` arrives, `own` when `from` is this user, its expiry stamped on the device's clock from `expiresInMs`                                                                |
| Starting         | On `transferStarting`, one peer connection with the frame's ICE servers, `iceTransportPolicy` `relay` for `relayOnly` and `all` otherwise; the sender opens an ordered, reliable data channel labelled `file` and offers, the receiver answers; candidates trickle through `transferSignal`, those arriving before the description held |
| Sending          | 64 KiB chunks, pausing while more than 4 MiB wait in the channel until it drains to 1 MiB (`bufferedamountlow`); progress shown at most every 200 ms and with every other change                                                                                                                                                        |
| Receiving        | Each chunk written as it arrives; more than the offered size fails the transfer; at the last byte the file is kept, `done` sent down the channel ahead of closing it, and `endTransfer completed` sent                                                                                                                                  |
| The verdict      | The sender completes on the receiver's `done`; a channel that closes with everything sent waits 5 s for it, then fails; one closing earlier fails at once                                                                                                                                                                               |
| The route        | Read from the connection's statistics (the transport's selected pair, or the nominated succeeded pair) on connecting and every 3 s                                                                                                                                                                                                      |
| Ending           | Cancel sends `endTransfer cancelled` and closes at once; `transferEnded` closes with its reason, `endedBy` the peer; a transfer that ends short of completion discards what was written; finished transfers stay until dismissed                                                                                                        |
| The call's place | `ready` resets offers, links, and the relay's policy and closes every transfer; leaving closes everything                                                                                                                                                                                                                               |

Where the engine differs, it is because the file lives on the platform and the engine runs without
the page:

- The bytes never cross the platform boundary whole. A file to offer is one the system's picker
  chose, held by the native side by reference and read a chunk at a time for each transfer of it
  (`files::FileSource`); a file received is written a chunk at a time into the place the picker
  chose before accepting (`files::FileSink`), so every receiver's transfer is to disk and none is
  held for the page. The platform lets a picked file go (`files::Files::release`) once its offer is
  gone and nothing sends it.
- An offer is made once the file has been opened for its size, so the offer states what the file is;
  a transfer of a file that has since changed, ending short of that size or going on past it, fails
  rather than deliver a file other than the one offered.
- What can fail before any transfer, which a browser's cannot, is `FilesState.failure`: a picked
  file or a chosen place that could not be opened, a server's `error` answering `offerFile` or
  `acceptFile` (named by the oldest such request not yet answered, since the server answers in
  order), a file command outside a call, or an offer gone while its place was being opened. The
  panel shows it with the file's name and the reason until dismissed or the next file command.
- An offer that has run out on the device's clock is taken down there too, not only on the server's
  `fileWithdrawn`; the timer is judged by the telling it was set for, so a later offer under the
  same id stands.
- A `ready` after a resume keeps the files of the offers it still lists as this user's, so an offer
  standing through a resume can still be served; a browser lets every offered file go on `ready`.
- A browser's end-of-candidates mark (an empty candidate) is never handed to the platform's WebRTC,
  which refuses it, nor sent.

### Saving the file

Where the receiver can choose a place first, accepting opens a picker and the file is written there
as it arrives (a `FileSink`). The file is closed only when every byte is in, and discarded when the
transfer ends otherwise.

| Platform                                                                   | How                                                                                                                                                                                            |
| -------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Browser with the File System Access API (Chromium, so the desktop app too) | Picker, written as it arrives                                                                                                                                                                  |
| Android                                                                    | `AspenFilesPlugin`: the system create-document picker names the file, which the call engine writes through `CallFiles` (`DocumentSink`, by the id `FileRegistry` holds it under)               |
| iOS                                                                        | `AspenFilesPlugin`: the system's save picker moves an empty file of that name where the user says, and the call engine writes there in place through `CallFiles` (`FileRegistry.ReceivedFile`) |
| Elsewhere (Firefox and Safari)                                             | Held until it has all arrived, then saved                                                                                                                                                      |

Offering on a phone starts from the system's open-document picker too (`AspenFilesPlugin.pick`),
which hands the page a reference, name, and size (`PickedFile`); the native side holds the file (a
security-scoped URL on iOS, a document URI with a persistable read permission on Android) for the
engine to read, and lets it go when the engine or the page releases it.

## The record

The deployment keeps a record of every offer and transfer for its moderators (`app::file_transfer`,
tables `file_offer` and `file_transfer`).

- It holds who offered what, by name and size, who received it, in which mode, and how it ended.
  Never the file.
- The voice server reports each offer (`VoiceReport::FileOffered`) and each transfer's start and end
  into the report stream (see [Voice](voice/index.md)).
- It names the offer by an id of its own (`record`, UUIDv7) rather than the id the client chose, so
  no client can make its offer collide with another in the record.
- A transfer's start is recorded once however many times its report arrives.
- A snapshot carries only calls, so a transfer report that is lost is missing from the record for
  good.

### Reading it

It tells who sent what to whom in private calls, so only holders of Moderate any community read it:

- `GET /admin/file-transfers`: newest offer first, one person's by `filter[user]` (as sender or
  receiver), a page of `limit` (50, at most 100) at a time. The answer is `{data, nextCursor}`:
  `nextCursor` is the `cursor` of the page after, the id of the page's last offer (`app::cursor`),
  and `null` on the last, which the read decides by asking for one offer more than the page holds;
- the dashboard's File transfers section, narrowed to one person by `AdminUserPicker` and widened
  again by its Anyone option or by emptying it.
