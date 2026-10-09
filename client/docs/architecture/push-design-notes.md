# Push: design notes

Rationale for [push.md](push.md).

## No push without a relay on Android

A build naming no relay registers for nothing, and `describe` refuses before the app asks.
Android's registration would ask a Firebase the build lacks and take the app down.

## Placeholder for `read` and `deleted` on iOS

A `read` or `deleted` pointer, and any failure, leaves the placeholder notification. The build
holds no filtering entitlement.

## Nothing in backups

Sessions and push keys are kept out of every backup and device transfer. They would sign
whoever restored them in as the user.
