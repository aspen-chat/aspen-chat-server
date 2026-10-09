# Threads and DMs

Threads are channels of replies to one message. DMs and group DMs are channels that belong to no community. The system account is how the deployment speaks to people, through a DM of its own.

## Pages

- [Threads](threads.md): how a first reply makes a thread, what it records, when an unreplied thread is removed, its reply summary, and what happens when its parent goes.
- [Echoes](echoes.md): echoing a thread reply to its parent channel, on posting or later.
- [Following threads](following-threads.md): who follows a thread, how, the cap, and what a follow tells.
- [DMs and group DMs](dms.md): making them, their people, listing them, who may be in one, and who may read one.
- [Calls in DMs](dm-calls.md): DM calls, rings, and the `call` and `missed_call` messages.
- [The system account](system-account.md): the deployment's own account, its name, and its notices.

Why things are the way they are: [design notes](design-notes.md).

## Key files

| Part | Where |
| --- | --- |
| Threads and echoes | `app::thread` |
| Following threads | `app::thread_follow`, table `thread_follow` |
| DMs and group DMs | `app::dm`, table `dm_recipient`, `channel.dm_key` |
| Access to DMs | `app::permissions::channel_access`, `channel_access_reading`, `channel_access_moderating` |
| Rings | `app::voice::ring`, table `voice_ring` |
| The system account | `app::system_account` |
