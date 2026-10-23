# The title bar and the page's title

The desktop app draws its window's title bar in the community rail's colour, holding the page's
title. The system still draws the window buttons. Every shell titles the page after where the
reader is.

## The page's title

`document.title` is set by `FollowDocumentTitle` (`src/features/layout/documentTitle.ts`),
mounted once at the root. Browsers show it on the tab, the system on the window and its taskbar
button, and the desktop app in its title bar.

- Each screen names itself with `useDocumentTitle(page)`. The screen mounted last is shown;
  `null` stands for a screen still loading.
- The title is `{page} · Aspen` (`documentTitle` in `src/i18n/messages.ts`), or `Aspen` alone
  while no screen names itself (signed out, the home redirect, an invite).
- It names where the reader is no more closely than a community: never a channel, a DM's people,
  or a thread.

| Screen                                                                 | Page                 |
| ---------------------------------------------------------------------- | -------------------- |
| A community (`ChannelSidebarLayout`), at home or on another deployment | The community's name |
| Direct messages (`DmLayout`)                                           | Direct messages      |
| Activity, Saved messages                                               | Their headings       |
| The Administration Dashboard                                           | Administration       |
| Open source attributions                                               | Its heading          |

## The desktop app's title bar

### The window (`packages/desktop/src/main/titleBar.ts`)

- The window is made with `titleBarStyle: "hidden"` and a `titleBarOverlay`, so the system hides
  its title bar and draws only the window buttons over the page's top edge (Chromium's Window
  Controls Overlay).
- Windows and Linux draw the buttons in colours the page sends (`title-bar:paint`). macOS draws
  its own traffic lights.
- The buttons are `STRIP_HEIGHT` (32px) times the zoom tall, but never less than `MIN_HEIGHT`
  (28px). `zoom.ts` refits them as the zoom changes. macOS sets the buttons' height once, as the
  window is made, so there the page's strip grows past them instead.
- `title-bar.json` in the profile keeps the last colours, so a window opens in the ones it closed
  in. On Linux it also keeps whether to draw the title bar at all.

### The page

- `followTitleBar` (`src/theme/titleBar.ts`) marks `<html>` with `data-title-bar` while the page
  is laid out under the buttons, and follows the overlay's `geometrychange`.
- While it is, `--title-bar-height` (`src/styles.css`) is the buttons' height
  (`env(titlebar-area-height)`), and never less than 2rem, plus 1px for the line along its foot,
  so the line runs below the buttons rather than under them. Everything else keeps clear of it:
  - `#root` pads its top by it;
  - every overlay (`overlayClass`, `Drawer`, `ImageGallery`, a screen share filling the window)
    starts below it, so a dialog leaves the title bar undimmed, as the system's own does;
  - the fallback toast region sits below it.
- `TitleBar` (`src/features/layout/TitleBar.tsx`) draws the strip: fixed across the top, in
  `bg-surface-rail`, ruled off from the page below by a 1px `border-line`, the colour of the
  channel list's dividing lines. The title is centred, in muted text, in the area beside the buttons
  (`titlebar-area-x` and `titlebar-area-width`, which are physical whichever way the page is
  written).
- The strip is hidden from assistive technology, which reads the document's title already.

### Dragging

- The strip is the window's drag region (`-webkit-app-region: drag`).
- A drag region takes every click in it, whatever is drawn over it. So everything rendered beside
  `#root` (popovers, menus, tooltips, modals) is `no-drag`, and a popover that opens over the
  strip can still be clicked.

### Colours

- `TitleStrip` reads the strip's computed background (the rail's colour) and ink, and sends them
  to the shell as `#rrggbb` for the buttons and their symbols.
- `hexColor` draws a computed colour on a canvas to read it as sRGB. The rail's colour is mixed in
  OKLab, and the shell takes only hex.
- The colours are read again whenever they may have changed:
  - the palette, the theme mode, and the contrast (attributes on `<html>`);
  - the system's light or dark preference, which `light-dark()` follows under the System mode;
  - forced colours, under which the strip is the theme's ground and the buttons follow.

### Keeping the system's title bar (Linux)

- On Linux, Settings › Appearance offers **Draw the title bar in Aspen's colours**
  (`TitleBarCheckbox`). It is on by default.
- With it off, the window keeps the title bar the desktop draws (KDE's decorations, or none under
  a tiling window manager), and the page draws no strip.
- A window's frame is fixed as it is made, so the choice applies from the next launch. Until
  then Settings says so and offers **Restart now** (`app.relaunch`).
- Windows and macOS always draw the title bar.

See the [design notes](title-bar-design-notes.md) for why it is built this way.
