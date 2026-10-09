# File transfers: design notes

Why [File transfers](file-transfers.md) work as they do.

## Offers

- An offer lasts at most `MAX_VALID_SECONDS` (an hour). An unattended offer then stays a thing for the people in a call, rather than a device seeding a file to whoever joins.
- The sender's client answers acceptances by itself, so an offer works while its sender is away.

## Network

- The voice server answers STUN itself, so no third party learns who is connecting.
- TURN credentials name the offer's `record`, the id the server made, never the client's id, and work from at most two addresses. One side then holds at most two allocations, and no credential can take the relay's ports.
- The relay forwards only between its own live allocations, so it never forwards to a port of its range on the machine that no allocation holds.
- The relay is looped back inside the server, so it needs no hairpin NAT.
- The pacer drops the odd datagram with rising probability past half full, rather than only dropping on overflow. A data channel's congestion control then settles near the limit rather than stalling on a run of losses.

## The record

- Offers are recorded by the server's own id (`record`) rather than the client's, so no client can make its offer collide with another.
- Only holders of Moderate any community read the record, since it tells who sent what to whom in private calls.
