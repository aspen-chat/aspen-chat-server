# Catalogue and federation

## `GET /plugins`

`GET /plugins` lists the plugins that are on. Each has:

- its name and description;
- its community settings fields;
- the permissions its principal asks for;
- whether it runs in DMs;
- its messages in the reader's language (a pseudo-locale is made from its default language);
- each kind of channel it declares (see [Channel types and views](channel-types-and-views.md#in-the-catalogue)).

Clients draw annotations, `alteredBy`, and settings forms from it.

## Federation

The deployment's document and `GET /auth/methods` name each plugin that is on as a capability (`Protocol::with_plugins`).
