# Context menus: design notes

Why [context-menus.md](context-menus.md) is the way it is.

## One list of actions, drawn three ways

A message's actions are offered by a row of buttons over it, by a touch screen's sheet, and by a
right click. Each action is one component, and `IconAction` draws it as the place asks (an icon with
a tooltip, a row with its name, a menu's item). A list of actions written again for the menu would
drift from the row: an action added to one and not the other, or offered under other conditions. A
person's actions are likewise one `UserMenuItems`, which every place that names someone opens, and
which a DM's row and a call's participant hold inside their own menus.

The row over a message keeps to what is done often (react, reply, save, pin, reactions, thread,
echo, edit) and ends with a button opening the menu, which has everything. With every action in it,
the row ran past the edge of a thread's pane at its first width, and of the channel beside it.

## One menu per thing clicked

A DM's row, and a person in a call, are both a place and a person. Two menus, one for each, would
make the reader guess which a right click opens. So each opens one menu: the place's own items
first, then the person's, parted by a rule.

## When the browser's menu is kept

Taking every right click would take away what the browser's menu is for: spelling suggestions in the
message box, copying what the reader selected, and a video's playback controls. So those keep the
browser's menu, and Shift held keeps it anywhere, for whatever else the reader wants of it, such as
saving a picture.

## Saving and copying read the storage's read path

Saving a picture, or copying it to the clipboard, needs the app to read its bytes. They are read
from the storage's read path itself, which every deployment's operators are told to open to any
origin (`docs/operators/installing/storage-read-path.md`), rather than through the API servers,
which would carry every save's bytes. The read path is public already and read without credentials,
so opening it to any origin gives nothing away. A read path set up before that is named as the
reason when a save fails, so the reader knows whose it is to fix.

## Collapse all in one request

Folding is limited to twenty in a burst for each user (`rate_limits.toml`), so folding every
category one request at a time would fold a large community partway and be refused the rest. The
server folds a community's categories in one request instead, bounded by
`MAX_CHANNELS_AND_CATEGORIES`, with one event naming those that changed.

## A category's mute, and channels kept unmuted

A category's mute is the server's own, read with each channel's category whenever a mute is asked
about, rather than a mute of each channel in it: that way channels added or moved in later are muted
too, and one moved out is not. A channel's own setting outranks it, which is how a reader keeps one
channel of a muted category speaking to them.

## Passing right clicks on

While a popover is open, the page behind it is inert. Without passing a right click on, one on
another message while a menu is open would close the menu and show the browser's menu over an empty
layer, and the reader would have to right-click again. Desktop apps open the next menu at once, and
so does this one.
