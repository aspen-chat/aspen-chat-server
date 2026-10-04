# Aspen plugins

A plugin is a small program a deployment's operator installs to change what the deployment
does: filter or rewrite what people post, look at what they post and say something about it, or
add a feature of its own. Plugins exist so that changing Aspen seldom takes a fork: a fork
drifts from Aspen and its fixes, and speaks a protocol other deployments may not know, while a
plugin rides on a maintained Aspen and declares what it adds.

This document is the design and the contract plugins and hosts keep. The interface between them
is `spec/plugin.wit` (the WebAssembly component interface, package `aspen:plugin`), and a
plugin's manifest is described by `spec/plugin_manifest.schema.json`. Phases 1 and 2, below,
are built (`docs/architecture/plugins.md` describes how); phase 3 is not.

## Who is trusted with what

Anyone may write a plugin, and an operator installs plugins they did not write, so a plugin is
untrusted code. It runs in a sandbox with nothing but what the host hands it, and it asks for
that in its manifest, which the operator reads and accepts when installing, as a person accepts
an app's permissions. A plugin that misbehaves can do what it was allowed to, and no more.

Some powers no plugin is ever given, whatever its manifest asks: credentials, sessions, and
signing in (passwords, second factors, tokens, recovery), deployment roles, federation keys and
gates, and bans from the deployment. A plugin that could reach these could take over accounts,
and a sandbox cannot make that safe, since the danger is the power itself. Features of that kind
belong in Aspen.

Everything a plugin learns, it learns from the host: the record a hook is about, and what host
calls return. Every read the host answers is decided as Aspen decides it for a person (below),
never by a plugin's own reckoning, so a plugin cannot be made to show someone what they may not
see.

No plugin code runs in a client. Clients draw what plugins contribute from structured records
(annotations, `alteredBy`, settings forms), and a plugin's own views run in sandboxed frames
(phase 3). People of other deployments use this deployment's plugins through their own clients,
which is safe only because nothing a plugin sends a client is code the client runs with the
person's session.

## A plugin

A plugin is a WebAssembly component (the component model, built against `spec/plugin.wit`)
together with a manifest, a JSON file beside it:

- `id`: a domain its author controls, reversed (`org.example.nocursing`), as a fork names its
  capabilities (`spec/federation.md`). Aspen's own plugins use `org.aspenchat.*`.
- `version`: semver, for people and for the host's upgrades.
- `api`: the version of the plugin interface it was built against (`1`). The host supports every
  version of the last thirty-six months; within a version the interface changes only by
  addition, as the federation protocol does.
- `component`: the component's file name, beside the manifest.
- `name`, `description`: keys of its own `messages`, so they are shown in each reader's language.
- `author`, `homepage`: for the operator.
- `defaultLanguage` and `messages`: its text in each language it speaks (`{"en": {"key":
  "text"}}`), keyed as `server/locales/en.yml` is, with `%{name}` for what is filled in. Every
  key it names anywhere is in its default language. What it says to people (a refusal, an
  annotation, a setting's label) is always a key, never text, so it reads in each reader's
  language.
- `permissions`: what it may do (below). A permission not declared is refused at run time.
- `hooks`: which hooks it answers (`message.create`, `message.edit` to intercept; the events it
  observes), and for each intercepting hook its failure policy (below).
- `settings` and `communitySettings`: the fields the operator, and each community that turns it
  on, configures (below).
- `hosts`: the hosts it may call over HTTPS, when it holds `network`.
- `storageQuota`: the bytes of storage it may keep, when it holds `storage`.
- `attachmentLimit`: the largest attachment it may read, when it holds `attachments.read`.
- `principal`: its own account (below), when it holds `act`: the account's username, display
  name key, and the community permissions it asks for where it is turned on.
- `retention`: a key of its `messages` saying what it keeps of what it sees and for how long,
  which the operator reads before installing, since deleting a message cannot reach a copy a
  plugin keeps.

A plugin is a WebAssembly component because that is a hard sandbox with nothing reachable but
its imports, because the host meters it (a deadline for each call, a memory ceiling), and
because it can be written in any language that targets it (Rust, Go, JavaScript, Python, C). It
runs inside every API server, so a hook on the path of posting a message costs a function call,
not a round trip. A component built for `wasm32-wasip2` imports the WASI interfaces its standard
library uses; the host provides them with nothing behind them (no files, sockets, environment,
arguments, or clock beyond the time of day), so they reach nothing.

## Where a plugin runs

The operator installs a plugin for the whole deployment and decides how far it reaches:

- `everywhere`: its hooks run in every community. Communities may configure its community
  settings but not turn it off. For what the deployment must hold everywhere, such as a filter
  the law requires.
- `optIn`: its hooks run in the communities that turn it on. A member holding the community
  permission Manage plugins turns it on, configures it, and turns it off.

A DM belongs to no community. A plugin's hooks run in DMs only when the operator grants it the
separate permission `dms`, which the manifest must ask for and the operator must accept by name;
while any plugin holds it, every DM says that a plugin of the deployment can read DMs, naming
it. A thread runs where its parent channel does.

## Permissions

- `messages.read`: be shown messages (their text, author, place, and attachments' records) in
  the hooks it answers. Every message hook needs it.
- `messages.rewrite`: change a message's text before it is saved.
- `messages.refuse`: refuse a message before it is saved, saying why.
- `messages.annotate`: attach annotations to messages.
- `users.annotate`: attach annotations to people's profiles.
- `attachments.read`: read the bytes of attachments of messages it is shown, up to
  `attachmentLimit`.
- `dms`: run in DMs (above).
- `network`: call the `hosts` it lists, over HTTPS, through the host's outbound client, which
  reaches public addresses only and follows no redirects (as calls to other deployments do).
- `storage`: keep data of its own, up to `storageQuota`.
- `routes`: answer requests at `/api/v1/plugins/{id}/routes/…`.
- `events`: publish the `pluginEvent` event.
- `act`: have an account of its own, its principal, and act through it.
- `channelTypes`, `views`: phase 3.

A manifest that asks for a permission this host does not know is refused at install, saying the
plugin needs a newer Aspen.

## Settings

A plugin's `settings` (configured by the operator) and `communitySettings` (configured by each
community that turns it on, or for an `everywhere` plugin, each community that wants to) are a
list of fields, each with a `name`, a `label` and optional `description` from its `messages`, a
`default`, whether it is `required`, and a `type`:

- `boolean`; `integer` (with `min` and `max`); `text` (one line, with `maxLength`); `longText`;
  `textList` (one entry a line, with `maxItems`); `choice` (one of `options`, each a value and a
  label key).
- In community settings only: `role` and `roleList`, `channel` and `channelList`, which must
  name the community's own.
- `secret: true` on a `text` field (an API key) keeps it out of every read and every log: reads
  say only whether it is set, and the plugin alone receives it.

Settings are checked against their fields when they are written, a refusal naming the field and
why, and clients draw a form from the fields with no code of the plugin's. Who may read a
community's settings is who may change them: holders of Manage plugins.

## The host

Each call runs in a fresh instance of the component, so nothing of one call survives into the
next except what the plugin keeps through the host (storage, counters). Each call has a deadline,
and an instance a memory ceiling, both the operator's to set: an intercepting call has a few
milliseconds by default, an observing call or a route some seconds. A call that runs out of time
or memory, or traps, has failed (below), and is logged with the plugin's id.

Every call is told its context: the community it runs for (if any) and that community's settings,
and the locale of the person it serves, when there is one. The deployment settings are read with
`settings`. `log` writes to the server's log under the plugin's id.

**Counters** are what a plugin counts across calls and across the deployment's servers, in
Valkey: `counter-add(key, window-seconds, amount)` adds to the key's count in the current window
of that many seconds and returns the count, so a rule such as "five mentions in ten seconds" is
one call. Windows are fixed, aligned to the epoch. Counting needs no permission; each plugin's
keys are its own.

## Hooks

### Intercept, before saving

`message.create` and `message.edit`, more by addition. The host calls the plugin with the draft:
its text, author (with their roles in the community), place, and attachments' records, before
the transaction that saves it opens, so a slow plugin holds no database connection or lock. The
author's permission to post is checked first; what they may not post never reaches a plugin.

The plugin answers `allow`, `rewrite` with new text (with `messages.rewrite`), or `refuse` with a
reason from its `messages` (with `messages.refuse`), which the person reads, in their language,
as the refusal's detail (`pluginRefused`). Plugins that intercept the same hook run in the
operator's order, each seeing what the one before left; a refusal ends it. A rewrite never adds
tags: tags are read from the text as saved, and a rewrite that would tag someone or something
the author's text did not is refused (`pluginRewriteTagged`, failing the plugin as below).
Commands and moderators' warnings are not intercepted, and a principal's own messages are not
intercepted by its own plugin.

A rewritten message is marked: its record carries `alteredBy`, the ids of the plugins that
changed it, so every client, this deployment's or another's, can say so beside it, and its
author learns their text was changed by what, not only that it was. The original text is not
kept.

Each intercepting hook declares what happens when a call fails or runs out of time: `open`, as if
it allowed, or `closed`, as if it refused, with the host's own reason (`pluginUnavailable`). A
filter that must hold fails closed; one that only improves things fails open.

### Observe, after saving

After a record commits, the host hands its event to each plugin that observes it: `message.create`,
`message.edit`, `message.delete`, `command.invoke` (a command sent to its principal), and
`plugin.enable` and `plugin.disable` (a community turned it on or off). Observing is where slow
work goes: reading an attachment, calling a service, acting.

Each plugin reads the event stream through a durable JetStream consumer of its own, shared by
the API servers as the push dispatcher's is, so each event reaches a plugin once however many
servers run. Delivery is at least once (a call that fails is tried again, up to three times) and
only while the stream retains the event (`MAX_EVENT_AGE`, a minute): a plugin that is down for
longer misses what happened meanwhile. Events of one channel usually arrive in order but are not
guaranteed to; each carries its `eventId` and the time it happened, for a plugin that must tell.

### Annotations

An annotation is what a plugin says about a message or a person for people to see: its plugin,
a `kind` of its own, a severity (`info`, `notice`, `warning`), a label and an optional detail
(keys of its `messages`, with arguments), and an optional link (`https` only). A plugin keeps at
most one annotation of each kind on each message or person, and setting one replaces it.

Annotations on a message (`messages.annotate`) are records of their own, `messageAnnotation`,
published as events in the message's channel and sideloaded with `include=annotations`
(`included.messageAnnotations`), so whoever may read the message sees them and nobody else does.
Annotations on a person (`users.annotate`), `userAnnotation`, reach whoever shares a community
with them, as their profile does, and are read with `GET /users/{user}/annotations`. Clients
draw them the same way for every plugin, beside the message or on the profile, from the
plugin's catalogue (`GET /plugins`: each plugin's name, description, version, and messages in
the reader's language), so a client needs no code of the plugin's to show what it found. A
SynthID checker, say, observes `message.create`, reads image attachments, asks its service, and
annotates what it finds. Removing a plugin removes its annotations.

## Extending

### The principal

A plugin holding `act` has an account of its own, its principal: a bot that no person owns,
marked as the plugin's, which signs in to nothing and acts only through the host. It is made when
the plugin is installed and named by the manifest. Where a community turns the plugin on, the
principal joins it, with a role of its own holding the permissions the manifest asks for that
the person turning it on holds and grants, as adding a bot does (Add bots besides Manage
plugins); turning it off removes the principal and its role. For an `everywhere` plugin a
community grants its principal permissions the same way, if it wants it to act there.

The principal acts through the same paths as anyone (`send-message`, `delete-message`,
`remove-member`, `ban-member`, `add-reaction`), so its permissions, its rank, and everything that
takes access away from a member hold for it too: a community that removes it or takes its role's
permissions stops it at once. It may also publish commands, as any bot does, which reach the
plugin as `command.invoke`. Actions taken while intercepting are run once the hook has answered,
and return nothing.

### Storage

A key-value store per plugin (`storage`), in the database so every API server sees the same,
within `storageQuota`. Each value lives in a scope: the deployment, a community, a channel, or a
user, so that what a plugin keeps about a place goes with it: deleting a channel, community, or
account deletes what plugins kept in its scope, and removing a plugin deletes all of its data
once the operator purges it.

### Routes

Endpoints under `/api/v1/plugins/{id}/routes/{path}` (`routes`), authenticated as every
endpoint is and rate limited like any other. The plugin is handed the method, path, query,
body (at most 64 KiB), the caller, and their locale, and answers with a status, a content type,
and a body. While it answers a route, every read it makes through the host is made as the caller:
a message the caller may not read is not found, and storage in a channel's or community's scope
is readable only by those who may view the channel or belong to the community, and in a user's
scope only by that user.

### Events

The custom event `pluginEvent` (`plugin`, `kind`, a JSON `payload`), published to a channel, a
community, or a user as the plugin says (`events`), and routed like any event, so who may view a
channel decides who receives its plugin's events there. A plugin's events never change who may
see or do anything.

### Phase 3

- **Channel types**: a type named under the plugin's id (`org.example.forums:board`), whose
  permissions are the ordinary channel permissions and whose contents are the plugin's. A client
  shows a channel of a type it has no view for as needing that plugin.
- **Views**: a plugin's user interface, served by the deployment from an origin of its own (not
  the app's), run in a frame sandboxed without `allow-same-origin`, and talking to the app only
  through a bridge of messages that offers what the manifest was granted: the person's name and
  locale, the app's theme (colours, fonts, light or dark, direction), calls to the plugin's own
  routes, its events. The frame never holds the person's session token.

Later, as features like an event calendar need them: timers (a durable callback at a given
time), notifications (through Aspen's own, respecting mutes, to those who may view the channel
when sent), typed cards in messages with buttons that call the plugin's routes, and capability
URLs (a revocable secret per person, for a calendar feed).

## Federation

A deployment names each of its plugins as a capability in its document (`protocol.capabilities`,
by the plugin's id), so other deployments and their clients know which annotation kinds and
events to expect, and `GET /plugins` gives each plugin's version. Annotations and `alteredBy` are
ordinary fields and records every client reads, and clients ignore fields and kinds they do not
know, so a client of another deployment shows what it understands and passes over the rest.
Content of a DM follows the plugins of the deployment that hosts it.

## Operating plugins

Installing, upgrading, and removing a plugin, and granting its permissions, are terminal
commands, since running code is a power too strong for a web API
(`aspen-chat-server plugins install <manifest> | list | show | settings | mode | order | enable |
disable | remove | purge`). Installing shows the manifest's permissions, hosts, and retention,
which the operator accepts; `dms` is accepted by name. An upgrade that asks for more asks again.
The component and its settings are kept in the database, so every API server runs the same
version and picks up a change without a restart.

The Administration Dashboard lists installed plugins and, under the deployment permission
Manage plugins, changes their settings, mode, order, and whether they are on. A community's
settings have a Plugins section under the community permission Manage plugins.

## Phases

1. **Intercept, observe, annotate**: the WebAssembly host, manifests and permissions, settings
   at both levels, community enablement, `message.create` and `message.edit`, `alteredBy`,
   counters, annotations of messages and people and their client rendering, the terminal
   commands and the dashboard. Enough for a word filter and a SynthID checker. Built.
2. **The principal, storage, routes, events**. Enough for an automatic moderator. Built.
3. **Views and channel types**, with a forum as the proof, and what an event calendar needs.

## Open questions

- Whether plugins are signed by their authors, and whether the host checks a signature or only
  shows who signed.
