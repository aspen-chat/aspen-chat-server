# The game capture dialog

`GameCaptureDialog` is opened from `ShareControl`, the share button the call bar and the voice
channel header both use. It follows the platform.

## Windows and macOS

It lists the windows the capture source can be pointed at, with a checkbox for their sound.

## Linux

It lists the applications playing sound as a "Game audio" picker.

- It preselects the only one when just one plays, until the user picks.
- It lists them again twice a second while it is open, so an application that starts or stops
  playing appears or goes.
- A pick stands while its application is listed, and falls to no audio when it goes.
- Its button opens the system's picker for the picture.

### A known weakness

Choosing the sound and the picture separately is a known weakness, not a design goal. One choice of
"this game" is the better experience. It is out of reach only because the portal tells the app
nothing about the application behind the window the user picked.

Look for ways to close that gap:

- A video track's label names the window on X11, which could identify its application.
- A future portal may report the window's application.

## The test pattern

A development shell started with `ASPEN_TEST_MEDIA` naming a clip also offers it as a test pattern,
looped through libobs's media source with its sound. Without it no test pattern is offered. On Linux
the helper offers no pictures, so there is no test pattern there either.

A shell without the helper on disk shows no game option.

## Running the shell headlessly

The drives run the shell headlessly with:

| Variable | Effect |
| --- | --- |
| `ASPEN_DESKTOP_HIDDEN=1` | Runs the shell hidden. |
| `ASPEN_DESKTOP_FAKE_MEDIA=1` | Chromium answers `getDisplayMedia` itself with a synthetic screen, so no picker is shown. |
| `ASPEN_DESKTOP_USER_DATA` | Points at a scratch profile. |
