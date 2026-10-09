# Cards and notices

## Cards

`PluginCard` (`src/features/plugins/PluginCard.tsx`), drawn by `MessageBody`, shows a message's
`card` from its plugin's catalogue.

A card holds:

- a title;
- fields: a time in the reader's zone and language, a count, a person by `PersonName`, or a link;
- buttons, styled as the plugin asks.

### Buttons

- Each button is pressed through `AspenSync.pressCardButton`. It is disabled while on its way, and
  says when it failed.
- The press's answer is the plugin's to choose, so it is fetched as a plugin's route is
  (`AspenClient.pressCardButton`, through `#pluginFetch`):
  - it never refreshes or flags the session;
  - every failure reads the same, naming the plugin, whatever the answer says.
- The card's change arrives as the message's update.

A card of a plugin the catalogue lacks is not drawn, nor its buttons, where the message is still
shown.

## Notices

1. `AspenSync.onPluginNotice` hears `pluginNotice`. The server sends it only where the person's mute
   and level for the channel would tell of a message that tags them.
2. `NotifyOnMessages` tells of it as of such a message: its title is the plugin's name and its body
   the notice's text.
3. Clicking it opens its message, thread, or channel.

Phones are woken with the `notice` pointer (see [push](../push.md)).
