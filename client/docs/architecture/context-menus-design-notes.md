# Context menus: design notes

Why [context-menus.md](context-menus.md) is the way it is.

## One list of actions, drawn three ways

A message's actions are offered by a row of buttons over it, by a touch screen's sheet, and by a
right click. Each action is one component, and `IconAction` draws it as the place asks (an icon
with a tooltip, a row with its name, a menu's item). A list of actions written again for the menu
would drift from the row: an action added to one and not the other, or offered under other
conditions. A person's actions are likewise one `UserMenuItems`, which every place that names
someone opens, and which a DM's row and a call's participant hold inside their own menus.

## One menu per thing clicked

A DM's row, and a person in a call, are both a place and a person. Two menus, one for each, would
make the reader guess which a right click opens. So each opens one menu: the place's own items
first, then the person's, parted by a rule.

## When the browser's menu is kept

Taking every right click would take away what the browser's menu is for: spelling suggestions in
the message box, copying what the reader selected, and a video's playback controls. So those keep
the browser's menu, and Shift held keeps it anywhere, for whatever else the reader wants of it,
such as saving a picture.

## No Save or Copy image

Saving a picture, or copying it to the clipboard, needs the app to read its bytes. Attachments are
served from the deployment's object storage (`public_base_url`), which operators are told to open
to the page's origin by CORS for uploads only, so a read from the page may be refused on a
deployment configured as the operator guide says. A menu item that fails on some deployments and
not others is worse than none. Open original and Copy file address work everywhere, and Shift
with a right click offers the browser's Save image as.

## No Collapse all, and no category mute

Folding every category at once would be one request per category, and folding is limited to
twenty in a burst for each user (`rate_limits.toml`); a community with more categories would fold
partway and refuse the rest. Muting is the server's per channel; a category's mute would be a mute
of each of its channels, which would not follow channels added later. Both wait for the server to
offer them whole.

## Passing right clicks on

While a popover is open, the page behind it is inert. Without passing a right click on, one on
another message while a menu is open would close the menu and show the browser's menu over an
empty layer, and the reader would have to right-click again. Desktop apps open the next menu at
once, and so does this one.
