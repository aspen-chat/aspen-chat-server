# Intercepting

Code: `intercept::wanted`, `intercept::decide`, `intercept::decide_unchanged`.

## Messages

`app::message::create_message` and `update_message`:

1. Call `intercept::wanted`.
2. When any plugin running there intercepts the hook, check first:
   - the author's permission to post (`check_posting`);
   - that every attachment is their own ready upload or, on an edit, already the message's (`ensure_attachments_ready`).
3. Call `intercept::decide`, before the saving transaction opens.

### Edits

- An edit is decided whenever it changes the text or the attachments.
- One changing only the attachments shows the plugins the text as it stands.
- A rewrite of it is then saved as a change of text, its earlier rewriters still named.

## What a plugin may answer

Each plugin sees the text the one before left.

| Answer | Needs | Rules |
| --- | --- | --- |
| Rewrite | `messages.rewrite` | May not exceed the longer of the text and 4000 characters. May not tag anyone or anything the text did not (`tags_within`, by `mention::parse`). |
| Refuse | `messages.refuse` | Becomes `pluginRefused`, with the plugin's reason in the request's language. |

- A call that fails counts as allowing or refusing (`pluginUnavailable`), as its manifest's `failure` says.
- A rewrite or refusal without its permission counts as a failure.

## Commands

A command is decided as the text it shows (`bot_command::invocation_text`, checked first by `bot_command::check`), with the files it takes, by `intercept::decide_unchanged`. That turns a rewrite into a refusal (`pluginWouldRewrite`), since its arguments are what its bot receives.

## Polls

`app::poll::create_poll` and `write_in` decide under `message.create`, the same way, after the creator's or writer's permissions are checked:

- a poll, as its question, then each answer on a line of its own;
- a written-in answer.

No new hook exists for polls; see the [design notes](design-notes.md#polls-reuse-messagecreate).

## Not intercepted

- Warnings.
- The system account's notices.
- A principal's own messages, by its own plugin.

## Recording rewriters

The plugins that rewrote a message are stored in `message.altered_by` and carried as `alteredBy`. An edit replaces them, and its `Update` event carries the new set.

## Deferred actions

Actions a plugin asks for while intercepting (`host::Deferred`) run once the hook has answered (`principal::run_deferred`).
