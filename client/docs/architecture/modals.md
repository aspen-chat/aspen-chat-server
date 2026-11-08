# Modals

Every modal in the app is drawn the same way, from the class strings in
`src/features/invites/dialog.ts` and the title row in `src/features/layout/DialogHeading.tsx`. A
modal is React Aria's `ModalOverlay`, `Modal`, and `Dialog`: the overlay dims the page, the modal is
the frame, and the dialog is what the frame holds.

## The overlay

`overlayClass` covers the screen below the title bar (`title-bar.md`) and above whatever a drawn
keyboard hides (`--visible-top` and `--visible-bottom`, `src/theme/visibleArea.ts`), and centres the
frame in it with at least 1rem all round, more where the notch or home indicator reaches in
(`overlay-inset` in `src/styles.css`). Overlays stack in the order they open, each on
`overlayLayerClass`, the layer React Aria gives popovers, so a modal opened from a popover is drawn
over it.

## The frame

`modalFrameClass` is the frame every modal shares: as wide as the overlay leaves it up to the
modal's own width (`max-w-md` for `modalClass`, wider for a list, a grid of choices, or planes side
by side), never taller than the overlay, and scrolling within itself when its content is taller, so
that a long form on a phone scrolls while the page behind it stays where it is. The frame is the
scroll container, and its padding scrolls with its content. It keeps that padding, and the gap
`dialogClass` leaves between a dialog's rows, in the variables `--modal-pad` and `--modal-gap`
(`modal-frame` in `src/styles.css`), and names its own background as `--modal-ground`: a plain modal
is raised off the page, and a modal of planes (`planesModalClass` and its wider kinds) is the page's
ground with each plane raised off it.

## The title row

`DialogHeading` is every modal's title row: the title, the X in the top corner that closes the
modal, and on a later step of a stepped dialog an arrow before the title that goes back. The row
stays at the top of the frame while the modal scrolls (`pinned-title` in `src/styles.css`): it is
sticky within the frame, drawn on `--modal-ground`, and reaches over the frame's padding and the gap
below it, so what scrolls passes under it rather than showing through beside it, and the X is in
reach however far the reader has scrolled. A sticky row never leaves its parent's content box, so
the frame pads its top through the row rather than itself: were the frame padded at the top, the row
would stop that far below the frame's edge once scrolled, with what scrolls showing above it.
Whatever the frame scrolls into view, a control taking focus among it, lands below the row rather
than under it: the row measures itself (a `ResizeObserver`) into the frame's
`--pinned-title-height`, which the frame's `scroll-padding-top` reads, as does anything of a
dialog's own that stays in place while it scrolls (the role list of Community settings' Roles tab),
so it stays below the row. In a `Drawer`, which lays its title out with no frame variables set, the
same row lies where it is.

## Filling a touch-only screen

A modal that holds enough to fill a phone's screen anyway (Settings, Community settings, message
search, a new poll, Sign-in and security, Bots, About Aspen, Blocked users, Edit profile, the
reactions dialog, Top senders, Scheduled messages, and the dashboard's report resolution) fills a
touch-only screen edge to edge instead of leaving a margin and rounded corners around content that
is cut off anyway: it adds `touchFullScreenClass` to its frame class and puts
`fullScreenOverlayClass` on its overlay. A touch-only screen is one with a coarse pointer and no
hover, a phone or a tablet without a trackpad, whatever its size: the `touch-only` variant in
`src/styles.css`, the same query as `TOUCH_ONLY` in `src/features/layout/useMediaQuery.ts`. On one,
the frame takes the overlay's whole height and width with no border or rounding, and the overlay
pads only the notch and home indicator insets (`overlay-inset-edge`); on any other screen both are
what every modal has. A small modal, such as a confirmation or an invite, keeps its margin on every
screen.

`e2e/modals.spec.ts` checks that every modal's X is in its top corner, that the title row stays at
the top of a scrolled modal with its ground covering what scrolls under it, and in the phone
projects that the large modals fill the screen and a small one does not.
