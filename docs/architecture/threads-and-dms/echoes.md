# Echoes

An echo shows a thread reply in the thread's parent channel. It is a message of kind `thread_echo`.

## What an echo is

- It has no content of its own.
- `echoOf` names the reply.
- The reply's author is its author.
- It is a reference, not a copy: the client shows the reply's current content.
- An echo's content cannot be edited.
- An echo cannot start a thread.

## Making an echo

| How | Endpoint | Answers |
| --- | --- | --- |
| On posting | Post to the thread with `echoToParent` | (as posting does) |
| Later, by the reply's author | `PUT /messages/{message}/echo` | `201` when it makes the echo, `200` with the echo the reply already has |

Both take Send messages in the parent channel.

An echo made later:

- Is new in the parent channel, timed when it was made.
- Is made only of text a person wrote: `standard` and `command` replies.

## The reply's `echo` field

A reply names its live echo in `echo`.

- It is written in the transaction that makes the echo. The column's reference is checked at commit, since the echo is inserted after the reply that names it.
- Whenever it changes later, the reply's `update` announces it.
- At most one live echo names a reply (`message_echo_of`, a unique index over undeleted rows).

## Deleting

| Deleted | Effect |
| --- | --- |
| The reply | Its echo is deleted in the same transaction. The echo's deletion is announced first, so no client ever holds an echo whose reply is gone |
| The echo alone | Its reply's `echo` is cleared. The reply may then be echoed again |

## Who sees an echo

Echoing shows no one anything new. The echo is a message of the parent channel:

- Read, routed, and searched past as any echo.
- Its events are in the parent's scope. The reply's `update` is in the thread's.
- It shows a reply whose thread its readers already read.

Losing Send messages in the parent stops further echoes and leaves those made, as it leaves messages sent.
