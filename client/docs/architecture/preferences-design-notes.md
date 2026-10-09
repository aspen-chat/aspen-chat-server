# Preferences: design notes

Rationale for [preferences.md](preferences.md).

## No CSS zoom

The app does not zoom with CSS (`zoom` on the root, which iOS's `WKWebView.pageZoom` also is).
Under it:

- the page keeps its full width and scrolls sideways;
- React Aria's popovers and the app's own gestures measure in unzoomed pixels;
- the layout's breakpoints do not follow it.

The app scales its type sizes instead (`type-scale`), and on desktop the shell zooms the window
as a browser does.

## Zoom applied by the shell

The desktop shell keeps the zoom factor and applies it before the page loads, so nothing jumps
at launch.

## Palette and fonts outside `PreferenceStore`

The palette and the fonts are kept per install outside `PreferenceStore`, since they apply on
the sign-in screen too.
