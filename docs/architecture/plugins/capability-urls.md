# Capability URLs

Code: `capability`, table `plugin_capability`.

## Getting one

`capability-path`, while answering a route, gives the caller their URL for a name of the plugin's.

- It is the same each time.
- One person holds at most 100 names of one plugin's (`capability::MAX_PER_USER`).
- It is kept in `plugin_capability`, keyed by its secret. The secret is kept so the plugin can show it again.

## Following one

`GET /api/v1/plugins/{id}/capabilities/{secret}` (`capability::follow`):

- is anonymous;
- is limited per address and per secret;
- calls the plugin's route `aspen/capabilities/{name}` as the person;
- is refused once they are banned or deleted.

Reading as them, the plugin answers only what they may still see.

## Revoking

Every URL of a person's ends (`capability::revoke_all`) when they:

- change or reset their password;
- sign out everywhere else.

The plugin hands out a fresh one when next asked. **Why:** see the [design notes](design-notes.md#capability-urls).

The host's route paths are reserved; see [Routes](routes.md#the-hosts-paths).
