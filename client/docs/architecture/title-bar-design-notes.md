# The title bar: design notes

## Hidden title bar, not a frameless window

Electron could draw no frame at all (`frame: false`), leaving the page to draw the window buttons
too. Hiding only the title bar keeps the system's buttons, and with them:

- Windows 11's Snap Layouts on the maximise button;
- the buttons' placement and mirroring for right-to-left;
- what assistive technology and forced colours expect of them;
- the system menu, double-click to maximise, and the resize borders.

None of these has to be imitated, or kept up as the systems change.

## Overlays start below the strip

A dialog that covered the strip would dim the title bar, which a system's title bar never is. A
click on it would also drag the window rather than close the dialog, since the strip's drag region
takes every click in it. So overlays start at `--title-bar-height`, and the strip stays the window's
own.

## The strip grows with the zoom

The title is text, sized with the type scale, so it grows with the zoom. Buttons of a fixed height
would leave it clipped when zoomed in. So the shell sizes the buttons to the zoom, as the page's
strip is. The `MIN_HEIGHT` floor keeps the buttons large enough to hit when zoomed out.

## Linux may keep the system's title bar

On Linux the system's title bar is whatever the desktop draws: KDE's decorations, GNOME's, or none
under a tiling window manager. A title bar of Aspen's own looks foreign beside KDE's windows, and
adds a strip a tiling window manager's user did not ask for. So Linux offers the system's instead.
On Windows and macOS the buttons stay the system's, and apps commonly colour their own title bars,
so Aspen's sits well beside other windows there and is always drawn.

## Titles name no channel

A window's title is shown beyond the app: in window switchers, on the taskbar, in the list a screen
share is picked from, and to any program that lists windows. So the page's title names the community
the reader is in, never the channel or the people in a DM.
