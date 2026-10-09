# Publishing

## `publish_event`

`publish_event(state, conn, scope, event)` takes an `EventScope` naming the ids the caller has: a channel, a message, a session, a community, a membership, or a user.

- It resolves the rest on the caller's connection. So an event published inside a transaction can name rows that transaction wrote.
- Events are published before their transaction commits; see [Settling and resyncs](settling.md).

## `expected_kind`

`expected_kind` pairs every event variant with the kind of scope it must be published with.

- It is a match with no wildcard arm. An entity added without deciding its routing does not compile.
- A scope of the wrong kind is refused at run time.

## Events about a user

An event about a user (their profile) is published once to every community they belong to, and once to themselves.

- Every copy carries the same `Aspen-Event-Id` header, surfaced as `eventId` on the frame.
- The client drops repeats.
- The copies are bounded by `[limits] max_communities_per_user` (500 by default), enforced where memberships are made.

## `publish_events`

`publish_events` publishes several events in order, the same way.

1. It hands every copy to NATS on the server's one connection.
2. Only then does it wait for the acknowledgements.

The stream keeps them in the order given, while the caller waits one round trip for them all.

## Operator commands

Operator commands that change what someone may do (`admin`, `communities set-owner`) publish through `app::events::Publisher`. It is the same publishing, without a server context.
