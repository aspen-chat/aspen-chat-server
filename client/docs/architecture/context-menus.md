# Context menus

What a right click opens: a message's actions, a person's, a channel's, a category's, a community's,
and a listed message's, each by name, at the pointer. The reasons behind the choices here are in
[context-menus-design-notes.md](context-menus-design-notes.md).

## Where it lives

| Part                                                 | Where                                                                                                             |
| ---------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------- |
| State, and when the browser's own menu is wanted     | `useContextMenu`, `wantsNativeMenu`, `claimContextMenu` (`src/features/layout/contextMenuState.ts`)               |
| The popover, its point, and passing right clicks on  | `ContextMenu`, `MenuPoint` (`src/features/layout/ContextMenu.tsx`)                                                |
| Items and their look                                 | `MenuAction`, `MenuRule`, `ParentLabel`, `menuItemClass` and the rest (`src/features/layout/menu.tsx`)            |
| Actions that are buttons elsewhere                   | `IconAction` inside `ActionsAsMenuItems` (`src/features/layout`)                                                  |
| Asking before what is hard to undo                   | `ConfirmModal` (`src/features/layout`)                                                                            |
| A message                                            | `MessageContextMenu`, `underPointer`, `saveFrom` and `copyPicture` (`mediaFile.ts`) (`src/features/messages`)     |
| A person                                             | `UserMenu`, `UserMenuItems`, `UserDialogs` (`src/features/users`), opened through `ProfilePopover`                |
| A channel or DM                                      | `ChannelMenu` (`src/features/channels`)                                                                           |
| A person in a call                                   | `ParticipantMenu` (`src/features/voice`)                                                                          |
| A category                                           | `CategoryMenu` (`src/features/channels`)                                                                          |
| A community on the rail                              | `CommunityMenu` (`src/features/communities`)                                                                      |
| A listed message (saved, activity, pins)             | `ListedMessageMenu` in `ListedMessage`, `EntryMenu` in `ActivityScreen`                                           |
| Mention, from a person's menu, and a message's Reply | `useComposerTarget`, `mentionIn`, `replyIn` (`src/features/messages/composerTargets.ts`), `useTagging`'s `insert` |

## Opening one

`useContextMenu` holds one menu's state. Its `onContextMenu` is the `contextmenu` handler of
whatever the menu is for. It opens the menu at the pointer, unless the browser's own menu is wanted
(`wantsNativeMenu`):

| The right click                                                    | Gets                                                    |
| ------------------------------------------------------------------ | ------------------------------------------------------- |
| With Shift held                                                    | The browser's menu, anywhere                            |
| In a text field (`input`, `textarea`, `select`, `contenteditable`) | The browser's menu, with spelling, cut, and paste       |
| On a playing `video` or `audio`                                    | The browser's menu, which controls it                   |
| Over text selected inside what was clicked                         | The browser's menu, to copy or look it up               |
| From a finger held down (`pointerType` `touch`)                    | Nothing of the app's: the touch screen's own long press |
| Already taken by something inside (`defaultPrevented`)             | That one's menu, not the one around it                  |

The last is how menus nest: a person's name inside a message opens the person's menu and not the
message's.

`claimContextMenu` makes the same choice for a right click that opens something other than a menu of
this kind: a community's name in the sidebar opens its settings, and the user's own name in the user
bar opens `PresenceMenu`, where it always opens.

The keyboard opens a menu too. The Menu key and Shift+F10 send a `contextmenu` to what has focus,
and the menu opens under its start with its first item focused. Browsers mark that event
differently, so the key is noted as it goes down (`menuKeyAt`), and Chromium's button of `-1`, a
pointer type of `""`, or a point of `0, 0` count too.

A menu that also has a button of its own (a channel's, a DM's, a person's in a call) opens beside
that button when it is pressed (`toggleFromButton`), and at the pointer when right-clicked.

## Drawing one

`ContextMenu` is a React Aria `Popover` holding a `Dialog`, placed against `MenuPoint`: an empty
element fixed at the point, at the end of the page so nothing that moves or scrolls places it. It
opens with its corner at the point (`bottom start`) and turns to whichever side has room.

- It is built from when it first opens (`opened`), and stays built so it can move as it closes,
  since each of a list's hundreds of rows may have one.
- Its items close it as they are chosen. A submenu's items close it themselves (`close`), since only
  the menu's own items do of themselves.
- What an item opens (a dialog, a picker, a profile card) is drawn outside the menu by its owner, so
  it outlives the menu: `MessageItem`'s sheets, `UserDialogs`, and each menu's own dialogs.
- A right click on the menu itself does nothing more: it is drawn inside what it is for in the React
  tree, which would otherwise open its menu again.

Every item is a `MenuAction`: its icon at the menu's size, then its name, in the danger colour for
what cannot be undone. The ID wizard's `CopyIdMenuItem` ends each menu while the wizard is on.

### Right-clicking one thing after another

While a popover is open React Aria makes the rest of the page inert and lays a layer over it, so a
right click elsewhere lands on that layer or on the page's body, whose menu would be the browser's;
and the popover may close as the button goes down, before the `contextmenu` arrives. One listener on
the document (`passOn`) takes a right click that lands behind any popover (a menu, a profile card),
closes every context menu still open, and sends it again to what is under the pointer once the page
is let out of being inert (`beneathPopovers`), so the next menu opens at once. With Shift held it is
left to the browser.

## A message

`MessageItem` opens `MessageContextMenu` on a right click on the message, while it is not being
edited.

1. The reader's quick reactions, where they may react, at a pointer's size (`QuickReactions`
   `compact`), with Add a reaction beside them.
2. What was right-clicked inside it (`underPointer`), when it is:
   - an attachment or a link's picture (`data-media-url`, on pictures, video posters, and file
     chips, with `data-media-name` and, on a picture, `data-media-picture`): Open original, Save
     picture or Save file, Copy picture (a picture, where the clipboard takes one), and Copy file
     address. Saving and copying read the file from the deployment's storage (`mediaFile.ts`), whose
     read path lets any origin read it (`docs/operators/installing/storage-read-path.md`); a picture
     is copied as a PNG. A storage that does not is named as the reason (`UnreadableFile`);
   - a link in its text that opens elsewhere (`a[target=_blank]`): Open link, which follows the link
     itself, and Copy link address.
3. Every action the message has (`MessageActions`), by name: those the row of buttons over it
   offers, and Mark unread, Copy text, Copy link, Report message, Copy message ID, and Delete
   message, which the row leaves to the menu so that it fits a thread's pane.

The row ends with More actions (⋯), which opens this menu beside it (`ContextMenu`'s `button`), from
the keyboard with its first item focused. The actions are the same components as the row of buttons
and the touch screen's sheet, which lists them all. `IconAction` draws each as an icon button, a row
of a list (`labelled`), or, inside `ActionsAsMenuItems`, a menu's item. What an action opens goes
through `MessageItem`'s `open` (`MessageSheet`), as the sheet's do, and the reaction picker is
placed at the menu's point.

## A person

`ProfilePopover` wraps every control that names someone (a message's author and picture, a tag, a
member of the member list, the DM header), so a right click on any of them opens their `UserMenu`. A
DM's row and a call's participant hold the person's menu in their own, and pass
`contextMenu={false}`.

`UserMenuItems` offers, each only where it would be allowed:

| Group           | Items                                                                                               |
| --------------- | --------------------------------------------------------------------------------------------------- |
| Reaching them   | Profile (their card, at the point), Message, Call, Mention, Copy username                           |
| Between the two | Change nickname (their own menu, in a community), Block or Unblock, Report profile, Report nickname |
| Moderation      | Roles (a submenu), Clear nickname, Remove from community, Ban from community, Ban from this server  |

- Message and Call go through `useReach`, as the profile card's buttons do. Neither is offered for
  the reader or the system account; Message is not offered inside their DM, nor to someone blocked
  unless the reader holds Message any user; Call is not offered for a bot or someone blocked.
- Mention puts their tag at the end of the message box of the channel the menu was opened in
  (`channelId`, else the route's), picked as though chosen from the list (`useTagging`'s `insert`),
  and focuses the box. It is offered only where that channel has a box open that the reader may post
  from (`useMentionTarget`), with Mention members.
- Roles lists the community's roles, highest first, each ticked while they hold it. Choosing one
  gives or takes it where the reader may (`useAssignableRoles`); the others are shown but not
  offered. It stays open, so several can be changed at once.
- Remove and Ban follow `useMemberModeration`, which the members panel uses too: Remove members or
  Ban members, over someone ranked below the reader who is not the owner. Removing asks first
  (`ConfirmModal`); banning opens `BanDialog`.
- Ban from this server opens `UserBanDialog`, for holders of Ban users, for a user of this
  deployment.
- Blocking asks first, as the profile card does; unblocking does not.

Whether they are a member, and so their roles, is read as the menu opens (`AspenSync.loadMember`)
when the store does not know.

## A channel or DM

`ChannelMenu` opens from a channel's row and a DM's row, at the pointer, or beside the row's options
button (`ChannelMenuButton`). Each channel's row has one, voice channels included.

- A voice channel: Join the call, Join muted (`VoiceCall.setMuted` before joining), or Leave call
  while the user is in it. Joining opens the channel too.
- Mark unread, Mute (which also keeps a channel unmuted inside a muted category), and Notifications,
  as [unread-and-muting.md](unread-and-muting.md) says.
- Copy link, the channel's link to share (`channelUrl`).
- Who can use this, Channel mana (a text channel's; `ChannelManaDialog`, with Manage channels),
  Rename channel, and Delete channel, for those who may.

A one-to-one DM's menu is the other person's (`UserMenu`), with these items first, named for the DM.

## A person in a call

`ParticipantMenu` is the person's `UserMenu`, named "Actions for", with the call's controls above
the person's items: their volume and their screen's (`CallVolumes`), then Mute for me, the same for
their screen's sound, and, with Manage calls, Server mute and Remove from call. On the reader's own
row it is their own menu alone. See [participants and volume](voice/participants-and-volume.md).

## A category

Right-clicking a category's heading opens `CategoryMenu`:

- Collapse or Expand category, as the heading does.
- Collapse all categories and Expand all categories, every one of the community's the reader may
  see, in one request (`AspenSync.setAllCategoriesCollapsed`).
- Mark as read, while one of its channels is unread (`useAnyUnread`, `AspenSync.markChannelsRead`).
- Mute, which mutes its channels that have no mute of their own (`CategoryMuteSubmenu`; see
  [unread-and-muting.md](unread-and-muting.md)), and Notifications, the level its channels without
  one of their own take (`NotificationsSubmenu`; see [notifications.md](notifications.md)).
- Add a channel (Manage channels), and Who can use this, Rename category, and Delete category
  (Manage categories), each opening the dialog the heading's buttons and the Layout tab use.

## A community on the rail

Right-clicking a community's icon opens `CommunityMenu`, inside the community's deployment
(`SourceScope`):

- Mark as read, while it is unread: every channel of it (`AspenSync.markChannelsRead`).
- Notifications, a submenu choosing what its channels without a setting of their own tell, as its
  settings do (`AspenSync.setCommunityNotifications`).
- Invite people, for holders of Create invites or Manage invites (`InviteModal`).
- Community settings (`CommunitySettingsModal`).
- Copy link (`communityUrl`).
- Leave community, for anyone but its owner, once they have confirmed. Leaving the community shown
  goes home.

A folder on the rail has its own menu (`FolderMenu`; see
[rail-and-channel-list.md](rail-and-channel-list.md)).

## Listed messages

A message listed out of its channel (`ListedMessage`: saved messages, the activity feed, a channel's
pins) opens `ListedMessageMenu`: Go to message, its row's own actions by name (Remove from saved,
Mark read, Unpin; each an `IconAction`), Copy text, and Copy link. The head of an activity entry
opens `EntryMenu`: Go to message, Mark read while it is unread, and Follow thread or Unfollow thread
for a thread's.

## Touch screens

A finger held down is left to the touch screen: a message's long press opens its sheet
(`MessageActionSheet`), rows have their options buttons, and a person's card has a button that opens
their menu (`MoreActions`, without Profile, which is where it is). Nothing here opens on a long
press.
