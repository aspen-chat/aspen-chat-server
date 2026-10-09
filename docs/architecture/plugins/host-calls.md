# Host calls

Code: `host::Call`.

`host::Call` checks the granted permission of every call.

## As whom

| Phase | Reads are made as |
| --- | --- |
| Answering a route (`Phase::Route`) | The caller of the route. |
| Otherwise | The plugin's principal. |

Never as the plugin itself.

## Reads

- `read-message` goes through `channel_access` for that reader.
- `read-message`, `place-of`, and `kind-of` are refused where the plugin does not run (`running_at`).
- `kind-of` gives a channel's type by its wire name (`ChannelType`'s `Display`) and `plugin_type`.

So a plugin answers only for channels of the kinds it serves. The example plugins each check the kind before reading or writing a channel's storage.

## Attachments

Attachments are readable when they belong to:

- what the call was shown; or
- in a route, a message the caller may read.

They are read only up to `attachmentLimit`, which bounds what is read from storage (`MediaStore::copy_object_to`):

- one object storage says is larger is refused before any of it is read;
- one that runs past the limit is dropped as it is read.

## `fetch`

- Reaches only the manifest's hosts, over HTTPS.
- Goes through a client that resolves public addresses only and follows no redirects.
- Refuses a host that is itself an address inside a network (`outbound::names_inside_address`), which the client would connect to without resolving.
- Never while intercepting.
- Bodies of at most 4 MiB.

## Storage

See [Storage](storage.md).

## Counters

`counter-add` keeps Valkey keys per plugin, window, and key, expiring with their window.

## Actions

`act` runs actions as the principal, through `app::message`, `app::react`, `app::role::remove_member`, and `app::ban::ban_member`.

- They run inside `settle_after`, as a request's would. One cut off at the deadline part way is still settled as failed.
- They are refused while the principal is banned from the deployment.

### Actions while answering a route

They are refused, as not found, beyond the caller's reach:

| Action | Refused when | Check |
| --- | --- | --- |
| `send-message`, `send-card` | The caller may not view the channel. | `caller_views` |
| `delete-message`, `add-reaction` | The caller may not read the message. | `caller_reads` |
| `remove-member`, `ban-member` | The caller does not belong to the community. | `caller_belongs` |
| `update-card` | Not limited. It changes only the principal's own card. | |
