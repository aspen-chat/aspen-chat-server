# Plugin settings

## In community settings

Holders of Manage plugins have a Plugins tab (`PluginsPanel`). It lists each plugin with whether it
runs there.

| Part | Name |
| --- | --- |
| Read | `useCommunityPlugins`, `GET /communities/{community}/plugins` |
| Topic | `communityPlugins:<communityId>` |
| Kept current by | `communityPlugin` events, which only holders of Manage plugins receive |

What it offers depends on where the plugin runs:

| Plugin runs | Offers |
| --- | --- |
| Where it is turned on | Turn on, with the permissions its account asks for ticked where the caller holds them (`AspenSync.enableCommunityPlugin`), and Turn off (`disableCommunityPlugin`). |
| Everywhere | Bringing its account in. |

Its settings are a `SettingsForm`.

## `SettingsForm`

`SettingsForm` (`src/features/plugins/SettingsForm.tsx`) draws any plugin's settings from the fields
it declares, labelled from its catalogue.

Field kinds:

- a checkbox;
- a number;
- a line or lines of text (a list is one entry a line);
- a choice;
- the community's roles or text channels.

Rules:

- A secret is never shown. Typing replaces it.
- Saving sends only the fields that changed, as a JSON Merge Patch. `null` restores a default.

## In the dashboard

The dashboard's Plugins tab (`PluginsSection`, `src/features/admin/Plugins.tsx`) is for holders of
View dashboard or Manage plugins. It lists the installed plugins in the order they decide messages
in, with:

- what each was granted;
- the hosts it may call;
- what it keeps;
- the storage it uses (`AdminApi.plugins`).

Holders of Manage plugins can:

- turn each on or off;
- choose where it runs;
- move it earlier or later (`orderPlugins`);
- change its settings (`updatePlugin`).

Installing, upgrading, and removing are the server's terminal's alone, which the tab says.
