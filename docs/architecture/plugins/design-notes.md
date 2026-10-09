# Plugins: design notes

The reasons behind the choices described in [Plugins](index.md). `spec/plugins.md` holds the design of the plugin interface itself.

## Running

### Places kept for intercepting

- **A quarter of each set of places is kept for intercepting calls.** Route requests and observers run for seconds. Without kept places they could take every place and make a message's intercepting calls fail, which a filter whose `failure` is `open` would let through. See [Admission](running.md#admission).
- **Host calls live on `host::Call`, apart from the store's WASI state.** What they hold across an await must be shareable between threads. See [Running](running.md#wasi).

## Intercepting

### Polls reuse `message.create`

- **Polls and write-ins are decided under `message.create`.** Reusing it keeps every installed plugin's filter covering polls without a new hook in the interface or the manifest. See [Polls](intercepting.md#polls).

### Commands

- **A rewrite of a command becomes a refusal (`pluginWouldRewrite`).** A command's arguments are what its bot receives. See [Commands](intercepting.md#commands).

## Observing

- **Where a plugin runs is decided from the event's subject alone, before anything else.** Every observer reads every event, so an event elsewhere costs nothing more. See [Where the plugin runs](observing.md#where-the-plugin-runs).
- **`first_copy` notes in Valkey which subject's copy of a DM event the plugin handles.** A DM's events come once per recipient's subject, so the plugin is told once and a redelivery of that copy is still handled. Community subjects carry one copy and need no note. See [DM events](observing.md#dm-events).

## Host calls

- **`place-of` and `kind-of` are refused where the plugin does not run.** A plugin answers only for channels of the kinds it serves. See [Reads](host-calls.md#reads).
- **Actions run inside `settle_after`.** One cut off at the deadline part way is still settled as failed. See [Actions](host-calls.md#actions).

## Storage

- **Quotas are counted per owner, not per plugin.** One community filling its share leaves every other's room. A plugin's total is summed from its shares when read, so no write waits on another owner's. See [Owners and quotas](storage.md#owners-and-quotas).
- **A scope must exist.** A community or channel not deleted, a person not deleted (and a member where the plugin is on), so a plugin cannot multiply its quota by naming owners that do not exist. See [Scopes](storage.md#scopes).
- **`plugin_storage.key` is in the `C` collation.** Keys compare by their bytes, so a prefix listing is a range of the index however much else the scope holds. See [Listing by prefix](storage.md#listing-by-prefix).

## Annotations

- **Removing a plugin deletes its annotations without events, in a job.** No read shows a removed plugin's annotations meanwhile. Clients draw none for a plugin `GET /plugins` does not list, so every client infers it from the catalogue. See [Remove and purge](installing.md#remove-and-purge).
- **`communityPlugin` carries `Aspen-Requires: managePlugins`.** Only the community's managers receive its settings. See [`communityPlugin`](annotations-and-events.md#communityplugin).

## Routes and views

- **Route answers that a browser would render or run become `application/octet-stream`, under a sandboxing CSP.** An answer is served on the API's origin. See [Answer headers](routes.md#answer-headers).
- **A view's CSP sandbox omits `allow-same-origin`.** It gives a page an opaque origin however it is opened. See [Serving the view](channel-types-and-views.md#serving-the-view).

## Timers

- **Timer jobs are keyed `plugin/key`.** A plugin's id is a reversed domain, so the key cannot be confused.
- **Setting a key again replaces its job under a new id.** A run of the timer it replaces then finishes without deleting the new one. See [Setting](timers.md#setting).

- **The per-owner timer count is a soft cap.** It is counted when a new key is set, and two set at once may each find room, so an owner may hold a few more than `MAX_TIMERS_PER_OWNER`. See [Setting](timers.md#setting).

## Capability URLs

- **Every URL of a person's ends when they change or reset their password or sign out everywhere else.** One copied by whoever took the account is a credential like a sign-in. See [Revoking](capability-urls.md#revoking).
