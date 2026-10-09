# Access checks

## Reads

- A read of one channel's contents checks `app::permissions::channel_access`. Its `ChannelAccess` nothing else can make.
- The readers that list what lies in several channels take an `app::visibility::Visibility` and keep only what it lets the caller view. These are channels, calls, read states, mutes, notification settings, and channel overrides.

## Refusals

| Caller | Answer |
| --- | --- |
| Not a member, or may not view the channel | `404`, as if it did not exist. |
| A member missing a permission | `403` `forbidden`, whose detail names the permission. |

## Sideloads

Community reads sideload `include=roles`, which gives `roles`, `channelOverrides`, and `categoryOverrides`. A channel's overrides come only for a channel the caller may view.

## Events

- Role, override, and membership changes are events on the community's subject.
- A channel override's event is routed by its channel's View channel, like the channel's own events, before and after the change (see [Event routing](../event-routing/index.md)).
- Someone who may not view a channel receives none of its events and finds it in no list.

## Calls

A call's join token carries `speak`, `shareScreen`, `useCamera`, and `transferFiles`. The voice server enforces them, and the API server keeps them current.

Every change to who may do what in a community rechecks the calls it touches (`app::voice::recheck`; see [Voice](../voice/index.md)):

- whoever may no longer be there is removed;
- everyone else is sent what they may now do.

## The client

The client follows the same changes (`AspenSync`).

| It hears of | It does |
| --- | --- |
| A channel it does not hold | Looks the channel up and reads its community again. |
| Deletion of a role the caller holds | Reads the community again. |
| The caller coming to moderate the deployment | Reads the community again. |
| `communityResync` | Reads the community again. |
| Leaving, removal, or a ban | Takes the whole community out of the store. |
