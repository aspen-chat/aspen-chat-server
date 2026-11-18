# Push: design notes

Rationale for [push.md](push.md).

## No push without a relay on Android

A build naming no relay registers for nothing, and `describe` refuses before the app asks. Android's
registration would ask a Firebase the build lacks and take the app down.

## Asking before the system does

The app asks whether to wake the device before the system's prompt, which a person cannot be shown
again once they refuse it on iOS: the app's own Decline leaves the system's question unasked, so
Settings can still turn push on later without sending the person to the system's settings. Nothing
the person pressed explains a prompt that comes at the app's start, unlike the camera's, which the
button that asks for it explains.

## Placeholder for `read` and `deleted` on iOS

A `read` or `deleted` pointer, and any failure, leaves the placeholder notification. The build holds
no filtering entitlement.

## Nothing in backups

Sessions and push keys are kept out of every backup and device transfer. They would sign whoever
restored them in as the user.
