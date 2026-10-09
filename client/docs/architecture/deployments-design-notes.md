# Deployments and federation: design notes

Rationale for [Deployments and federation](deployments.md).

## Sessions and preferences

- The home lists the other deployments. Every device of the user then signs in to the same ones.
- Every foreign `AspenSync` shares the home's `PreferenceStore`. Preferences are the user's, and are kept at home.

## Whose word is believed about identity

- Only the viewer's home is believed about where a user from elsewhere is from. The home checked the assertion the user signed in with, so its answer is the one backed by a check.
- Any other deployment's users from elsewhere are named by that deployment and their id there. A deployment could claim its user is someone else. If its claim were believed, a block of that user would hide the person it named.
- A person from a third deployment, blocked on a foreign one, is hidden on that one alone. This follows from the rule above: that block names them by the foreign deployment, not by their home.

## File names

- Offered and attached file names drop format and control characters. A bidirectional override could otherwise make `gpj.exe` read as `exe.jpg`.

## iOS plugin registration

- `MainViewController`, not a bare `CAPBridgeViewController`, is the window's root. `SceneDelegate` builds the window itself rather than from the storyboard, so the app's plugins are registered only where its own view controller registers them.
