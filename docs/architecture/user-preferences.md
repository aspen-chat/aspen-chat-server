# User preferences

A user's account-scoped preferences are one JSON object. `client/docs/architecture/preferences.md` describes the client store that holds both these and device-scoped preferences.

| Part | Where |
| --- | --- |
| Endpoints | `GET` and `PATCH /users/{user}/preferences` |
| Size limit | `app::preferences::MAX_PREFERENCES_BYTES` (the serialised size) |
| Event | Custom `userPreferencesChanged` |

## Access

Only the user themself may read or write them. Anyone else is answered `403`.

## Storage

- The server stores the object as the client wrote it.
- Keys are the client's, namespaced (`audio.input`, `look.theme`).
- Values are any JSON.
- The patch is a JSON Merge Patch at the top level, so a new setting needs no server change.
- Only the serialised size is policed (`app::preferences::MAX_PREFERENCES_BYTES`).

## Events

- A write publishes `userPreferencesChanged`, naming the user and the time but never the values.
- It goes to the user's own subject alone (`EventScope::User`).
- The user's other devices fetch on seeing it.

## Device-scoped preferences

Device-scoped preferences (which microphone to use, say) never reach the server.
