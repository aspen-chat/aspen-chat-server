# The directory

Every deployment this one knows is in the dashboard's Federation tab, with:

- its key's fingerprint,
- the lists it is on,
- whether it is admitted each way.

Administrators with Manage federation add deployments, put them on lists, and contact them there.

## From the terminal

```
aspen-chat-server federation status
aspen-chat-server federation list
aspen-chat-server federation add friends.example.net --note "The book club"
aspen-chat-server federation list-add friends.example.net usersEmigrationAllow
```

## Deployments recorded on contact

A deployment is also recorded the first time it is in contact, as when one of its people signs
in here.

One recorded that way that nobody uses is forgotten thirty days after it was last contacted,
unless it is waiting for you to accept a new key. "Nobody uses" means:

- no one of yours uses it,
- none of its people are here,
- it is on no list,
- it has no note.

Give it a note to keep it.

## Forgetting a deployment

`aspen-chat-server federation remove <domain>` (or Forget in the dashboard) forgets a deployment,
its key, and the lists it is on.

**A deployment on a block list cannot be forgotten**, since that would take it off the list and
let it in. Take it off its block lists first if you mean to.
