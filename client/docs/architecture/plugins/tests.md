# Plugin tests

`e2e/plugins.spec.ts` checks:

- an annotation arriving, and its popover;
- a changed message's mark;
- the DM notice;
- turning a plugin on with its account's permissions;
- a channel of a plugin's kind showing a stand-in page that talks over the bridge, and passes axe.
  The page checks:
  - its hello;
  - a route;
  - a path beyond the routes refused;
  - a person named;
  - only its own events;
- nothing asked on the frame's window answered;
- the bridge falling silent when the view goes to another page (which asks at once, before its own
  load) until it is loaded again;
- a kind no plugin declares;
- a card's fields and button;
- a notice's system notification.

The test world answers `GET /plugins` and people's annotations with nothing.
