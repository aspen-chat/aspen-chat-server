# Aspen plugins

A plugin is a small program a deployment's operator installs to change what the deployment
does: filter or rewrite what people post, look at what they post and say something about it, or
add a feature of its own. Plugins exist so that changing Aspen seldom takes a fork: a fork
drifts from Aspen and its fixes, and speaks a protocol other deployments may not know, while a
plugin rides on a maintained Aspen and declares what it adds.

This document is the design and the contract plugins and hosts keep. The interface between them
is `spec/plugin.wit` (the WebAssembly component interface, package `aspen:plugin`), and a
plugin's manifest is described by `spec/plugin_manifest.schema.json`. Every phase below is
built (`docs/architecture/plugins.md` describes how).

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
(Views, below). People of other deployments use this deployment's plugins through their own clients,
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
- `assets`: a directory beside the manifest whose files (pages, scripts, styles, pictures)
  are installed with it and served as its views' files, with `views`.
- `channelTypes`: the kinds of channel it adds, with `channelTypes`, by name: each one's name (a
  key of its `messages`), its `view` (a page among its assets), and its `glyph` (`board`,
  `calendar`, `list`, or `chat`), which clients draw it with.
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

- `messages.read`: be shown messages (their text, author, place, and attachments' records,
  descriptions included) in the hooks it answers. Every message hook needs it.
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
- `views`: serve pages of its own, its `assets`, to people's apps (Views, below).
- `channelTypes`: add the kinds of channel its manifest declares, each shown by one of its views.
- `timers`: be called back at times it sets.
- `notify`: tell people of something, as Aspen tells them of a message.
- `capabilities`: give a person a private URL of their own to one of its routes.

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
author's permission to post is checked first, and that every attachment is their own upload (on
an edit, or already the message's); what they may not post never reaches a plugin.
`message.edit` is called for every edit, of the text, the attachments, or both, with the text as
it will stand; on an edit of the attachments alone, a rewrite of that text changes it too.

The plugin answers `allow`, `rewrite` with new text (with `messages.rewrite`), or `refuse` with a
reason from its `messages` (with `messages.refuse`), which the person reads, in their language,
as the refusal's detail (`pluginRefused`). Plugins that intercept the same hook run in the
operator's order, each seeing what the one before left; a refusal ends it. A rewrite never adds
tags: tags are read from the text as saved, and a rewrite that would tag someone or something
the author's text did not is refused (`pluginRewriteTagged`, failing the plugin as below).
A poll is intercepted by `message.create` too, as its question followed by each answer on a
line of its own, and so is an answer written in to a poll, as its text; neither can be rewritten,
so a rewrite refuses them. A command is intercepted by `message.create` as the text it shows (`/name` and its arguments)
with the files it takes; its arguments are what its bot receives, so a rewrite that would change
the text refuses the command instead (`pluginRefused`). Moderators' warnings are not intercepted,
and a principal's own messages and commands are not intercepted by its own plugin.

A rewritten message is marked: its record carries `alteredBy`, the ids of the plugins that
changed it, so every client, this deployment's or another's, can say so beside it, and its
author learns their text was changed by what, not only that it was. The original text is not
kept.

Each intercepting hook declares what happens when a call fails or runs out of time: `open`, as if
it allowed, or `closed`, as if it refused, with the host's own reason (`pluginUnavailable`). A
filter that must hold fails closed; one that only improves things fails open.

### Observe, after saving

After a record commits, the host hands its event to each plugin that observes it: `message.create`,
`message.edit` (its text or its attachments changed), `message.delete`, `command.invoke` (a command sent to its principal), and
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
scope only by that user. So it is with what it sends: an event published, or a notice sent, while
answering goes only to a channel the caller may view, a community they belong to, or themself.

### Events

The custom event `pluginEvent` (`plugin`, `kind`, a JSON `payload`), published to a channel, a
community, or a user as the plugin says (`events`), and routed like any event, so who may view a
channel decides who receives its plugin's events there. A plugin's events never change who may
see or do anything.

### Channel types

A type named under the plugin's id (`org.example.forums:board`), which someone who may manage
channels makes like any channel where the plugin runs (`POST /channels` with `ty: "plugin"` and
`pluginType`). Its permissions are the ordinary channel permissions: View channel decides who
sees it and its contents, overrides apply, and a plugin decides what the others mean for it
(Send messages, say, for posting to a board). Its contents are the plugin's, kept in storage
scoped to the channel, so they go with it; it holds no messages of Aspen's own. A client shows it
by its type's view, and one of a type no plugin running there declares as needing that plugin.

### Views

A plugin's user interface: a page among its `assets`, served by the deployment at
`/api/v1/plugins/{id}/assets/{path}`, with `Content-Security-Policy: sandbox allow-scripts
allow-forms allow-popups` (and no `allow-same-origin`), which gives it an origin of its own, opaque
and shared with nothing, however it is opened; it may load only its own assets and inline
scripts and styles, and may connect nowhere. A client shows it in a frame sandboxed the same way.
The frame never holds the person's session token; it talks to the app only through a bridge of
messages, each an object with `"aspen": 1`, over a `MessagePort` the app hands the page with
`hello`. The port belongs to the page it was handed to, so a page the frame goes to after (the
view followed a link, or was redirected) cannot use it, and the app answers nothing posted to its
window: once the frame loads anything else, the client closes the port and says nothing more until
it loads the view again, in a new frame, with a new port.

- The app says `hello` once, posted to the frame's window when its page has loaded, with the
  port as the message's one transferred port (`event.ports[0]`); everything after goes over the
  port, both ways, so a page's bridge listens for `hello` from the start (a script in the page
  does). Its `context` is the plugin, the view, the channel (and its name) and community it
  shows, the person (`id`, `name`, `displayName`), their `locale` and text direction (`dir`), the plugin's
  `messages` in that language, `apiBase` (where the deployment's API is, for the capability URLs
  it hands out), and the app's `theme`: its colours by token name
  (`surface`, `ink`, `accent`, and the rest), its two font stacks, and whether it is light or
  dark. It says `theme` again when any of that changes.
- The frame asks `request` (`id`, `method`, `path`, `query`, `body`), which the app makes to
  the plugin's route as the person and answers `response` (`id`, `status`, `contentType`,
  `body` as text; `status` 0 when the deployment could not be reached). A path that would leave
  the plugin's routes (a `.` or `..` segment) is answered 400 without being sent.
- The frame asks `users` (`id`, `ids`), which the app answers `users` with each person's `id`,
  `name`, and `displayName`, as it already holds them or reads them; one it cannot find is left
  out.
- The app passes on the plugin's `pluginEvent`s for the channel and community the view shows, as
  `event`.
- The frame may ask `open` with a channel or message the person may open, which the app opens.

### Timers

A plugin holding `timers` sets a timer by key (`set-timer`), due at a time with a payload of its
own, and cancels it (`cancel-timer`); setting a key again replaces it. When it falls due, any one
API server calls the plugin's `observe` with `timer-fired`, at least once: a call that fails is
tried again a minute later, three times at most. A plugin keeps at most 10,000 timers.

### Notices

A plugin holding `notify` tells someone of something in a channel where it runs (`notify`), with
text of its `messages` and optionally the message it is about. The host sends it only if they may
view the channel now, and their settings would tell them of a message that tags them there (it is
not muted, and their level for it is not "nothing"); it answers whether it did. They receive the
`pluginNotice` event, which their apps show as a system notification and which opens the channel
or message, and their phones are woken with the push pointer `notice` (`spec/push.md`). A notice
is kept a week, for phones to read (`GET /users/@me/plugin-notices/{notice}`), and goes with the
channel.

### Cards

A plugin's account may post a message with a card (`send-card`), and change or remove the card
on a message it posted (`update-card`): an optional title and fields (each a label of its
`messages` and a value: text, a time each client shows in its reader's time zone and language, a
count, a person, or a link), and buttons (each an id of the plugin's, a label of its `messages`,
and a style). A card is part of its message, which carries it as `card` (naming its plugin), and
an update is announced as the message's. Pressing a button (`POST
/messages/{message}/card/buttons/{button}`), which takes being able to read the message, calls
the plugin's route `aspen/cards/{message}/{button}` as the person who pressed it. Nobody else's
message carries a card.

### Capability URLs

A plugin holding `capabilities` gives the caller of one of its routes a private URL of theirs
(`capability-path`, by a name of the plugin's, such as `feed:{channel}`), which reaches it
without signing in, for a calendar app's feed and the like:
`/api/v1/plugins/{id}/capabilities/{secret}`. A request to it calls the plugin's route
`aspen/capabilities/{name}` as that person, so it answers only what they may still see, and stops
answering at all once they are banned or their account is deleted. The person's URL for a name is
the same each time the plugin asks for it, until the plugin revokes it (`revoke-capability`).

Routes under `aspen/` are the host's to call: a person's own requests never reach them. A
person's request whose path has an empty, `.`, or `..` segment, or begins with `aspen` in any
case, as it stands or once percent-decoded again, is not found, so a plugin that normalises
or decodes its path is never led there.

Later: a view of a community's own, outside any channel, and plugins posting cards on messages
other than their own, if a need for either appears.

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
version and picks up a change without a restart. Removing a plugin stops it and takes its
account out of every community; its data stays until the operator purges it.

The Administration Dashboard lists installed plugins and, under the deployment permission
Manage plugins, changes their settings, mode, order, and whether they are on. A community's
settings have a Plugins section under the community permission Manage plugins.

## Phases

1. **Intercept, observe, annotate**: the WebAssembly host, manifests and permissions, settings
   at both levels, community enablement, `message.create` and `message.edit`, `alteredBy`,
   counters, annotations of messages and people and their client rendering, the terminal
   commands and the dashboard. Enough for a word filter and a SynthID checker. Built.
2. **The principal, storage, routes, events**. Enough for an automatic moderator. Built.
3. **Views and channel types, timers, notices, cards, and capability URLs**, with a forum and
   an event calendar as the proof. Built.

## Open questions

- Whether plugins are signed by their authors, and whether the host checks a signature or only
  shows who signed.
