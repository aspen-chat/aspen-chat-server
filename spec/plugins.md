# Aspen plugins

A plugin is a small program a deployment's operator installs to change what the deployment
does: filter or rewrite what people post, look at what they post and say something about it, or
add a feature of its own. Plugins exist so that changing Aspen seldom takes a fork: a fork
drifts from Aspen and its fixes, and speaks a protocol other deployments may not know, while a
plugin rides on a maintained Aspen and declares what it adds.

This document is the design and the contract plugins and hosts keep. None of it is built yet;
it is built in the phases at the end, each only when directed.

## Who is trusted with what

Anyone may write a plugin, and an operator installs plugins they did not write, so a plugin is
untrusted code. It runs in a sandbox with nothing but what the host hands it, and it asks for
that in its manifest, which the operator reads and accepts when installing, as a person accepts
an app's permissions. A plugin that misbehaves can do what it was allowed to, and no more.

No plugin code runs in a client. Clients draw what plugins contribute from structured records
(annotations, marks), and a plugin's own views run in sandboxed frames (Extending, below). People
of other deployments use this deployment's plugins through their own clients, which is safe only
because nothing a plugin sends a client is code the client runs with the person's session.

## A plugin

A plugin is a WebAssembly component (the component model, its interface described in WIT)
together with a manifest:

- `id`: a domain its author controls, reversed (`org.example.nocursing`), as a fork names its
  capabilities (`spec/federation.md`). Aspen's own plugins use `org.aspenchat.*`.
- `version`: semver, for people and for the host's upgrades.
- `api`: the version of the plugin interface it was built against. The host supports every
  version of the last thirty-six months; within a version the interface changes only by
  addition, as the federation protocol does.
- `name`, `description`, `author`, `homepage`: for the operator, each with translations.
- `permissions`: what it may do (below). A permission not declared is refused at run time.
- `hooks`: which of the host's hooks it answers, and for each, its failure policy (below).
- `settings`: a JSON Schema of what the operator configures (`words`, `threshold`), with
  `secret` fields (an API key) kept out of logs and out of every read but the plugin's.
- `messages`: its text in each language it speaks, keyed as `server/locales/en.yml` is, for
  what it says to people (a refusal, an annotation).
- `hosts`: the hosts it may call over HTTPS, when it holds `network`.

A plugin is a WebAssembly component because that is a hard sandbox with nothing reachable but
its imports, because the host meters it (fuel for CPU, a memory ceiling), and because it can be
written in any language that targets it (Rust, Go, JavaScript, Python, C). It runs inside every
API server, so a hook on the path of posting a message costs a function call, not a round trip.

## Permissions

- `messages.read`: the text, author, channel, and attachments' records of messages it is shown.
- `messages.rewrite`: change a message's text before it is saved.
- `messages.refuse`: refuse a message before it is saved, saying why.
- `messages.annotate`: attach annotations to messages.
- `attachments.read`: read the bytes of attachments, up to a size it declares.
- `network`: call the `hosts` it lists, over HTTPS, through the host's outbound client, which
  reaches public addresses only and follows no redirects (as calls to other deployments do).
- `storage`: keep data of its own, up to a quota it declares.
- `routes`, `events`, `channelTypes`, `views`: extend Aspen (below).

## Hooks

### Intercept, before saving

The host calls the plugin inside the transaction that saves the record, before it commits:
`message.create` and `message.edit` first, more by addition. The plugin answers `allow`,
`rewrite` with new text, or `refuse` with a reason from its `messages`, which the person reads
as the refusal's detail. Plugins that intercept the same hook run in the operator's order, each
seeing what the one before left. A rewrite never adds tags: the host reads tags from the text
the author wrote, and a rewrite that would add one is refused.

A rewritten message is marked: its record carries `alteredBy`, the ids of the plugins that
changed it, so every client, this deployment's or another's, can say so beside it, and its
author learns their text was changed by what, not only that it was. The original text is not
kept.

Each call has a budget (fuel and wall time, a few milliseconds by default, the operator's to
raise), and each hook declares what happens when a call fails or runs out: `open`, as if it
allowed, or `closed`, as if it refused, with the host's own reason. A filter that must hold
fails closed; one that only improves things fails open.

### Observe and annotate, after saving

After a record commits, the host hands the event to each plugin that observes it. Delivery is at
least once, in order per channel, from a durable JetStream consumer per plugin, shared by the API
servers as the push dispatcher's is, so each event reaches a plugin once however many servers
run and none is lost to a restart. Observing is where slow work goes: reading an attachment,
calling a service.

An annotation is what a plugin says about a message for people to see: its plugin, a `kind` of
its own, a severity (`info`, `notice`, `warning`), a label and an optional detail from its
`messages`, and an optional link. Annotations are records of their own, published as events on
the message's channel and sideloaded with `include=annotations`, and clients draw them the same
way for every plugin, beside the message, so a client needs no code of the plugin's to show what
it found. A SynthID checker, say, observes `message.create`, reads image attachments, asks its
service, and annotates what it finds.

## Extending

- **Storage**: a key-value store per plugin, in the database so every API server sees the
  same, within the quota it declared.
- **Routes**: endpoints under `/api/v1/plugins/{id}/`, authenticated as every endpoint is, the
  caller handed to the plugin, rate limited like any other endpoint.
- **Events**: the custom event `plugin` (`plugin`, `kind`, a payload), published to a channel,
  community, or user as the plugin says, and routed like any event, so who may see a channel
  decides who receives its plugin's events there.
- **Channel types**: a type named under the plugin's id (`org.example.forums:board`), whose
  permissions are the ordinary channel permissions and whose contents are the plugin's. A client
  shows a channel of a type it has no view for as needing that plugin.
- **Views**: a plugin's user interface, served by the deployment from an origin of its own (not
  the app's), run in a frame sandboxed without `allow-same-origin`, and talking to the app only
  through a bridge of messages that offers what the manifest was granted: the person's name and
  locale, calls to the plugin's own routes, its events. The frame never holds the person's
  session token.

## Federation

A deployment names its plugins' additions as capabilities in its document (`protocol.capabilities`,
the plugin's id and version), so other deployments and their clients know which channel types,
annotation kinds, and events to expect. Annotations and `alteredBy` are ordinary fields of the
records every client reads, and clients ignore fields and kinds they do not know, so a client of
another deployment shows what it understands and passes over the rest. Content of a DM follows
the plugins of the deployment that hosts it.

## Operating plugins

Plugins are installed, configured, ordered, disabled, and removed from the terminal
(`aspen-chat-server plugins install <file> | settings | order | disable | enable | remove`), as
every power over the whole deployment first is, and later from the dashboard under a deployment
permission of its own. The component and its settings are kept in the database, so every API
server runs the same version and picks up a change without a restart. Installing shows the
manifest's permissions and hosts, which the operator accepts; an upgrade that asks for more asks
again. Removing a plugin keeps its data until the operator purges it.

## Phases

1. **Intercept, observe, annotate**: the WebAssembly host, manifests and permissions, settings,
   `message.create` and `message.edit`, `alteredBy`, annotations and their client rendering, the
   terminal commands. Enough for a word filter and a SynthID checker.
2. **Storage, routes, events**.
3. **Views and channel types**, with a forum as the proof.

## Open questions

- Whether plugins are signed by their authors, and whether the host checks a signature or only
  shows who signed.
- What a plugin sees of a DM, which no role or deployment power reaches today: likely nothing
  unless the operator grants it for that plugin explicitly, and people are told a plugin can
  read their DMs.
- Whether plugins may post messages themselves, and as whom (a bot of the plugin's, the system
  account).
