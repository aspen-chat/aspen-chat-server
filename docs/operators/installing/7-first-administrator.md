# Step 7: The first administrator

## Grant the top role

1. Create an account from the web client.
2. Where the API server runs:

   ```
   aspen-chat-server admin grant <username>
   ```

The command needs the database and NATS, as the servers do: it announces the change to the
account's open apps.

The account must be a person's on this deployment. Bots and users from other deployments hold no
deployment roles, and the command refuses them.

It gives the account the deployment's top role. If there is none, it makes an Administrator role
with every permission except the moderation ones:

| Moderation permission | What it allows |
| --- | --- |
| `moderateCommunities` | Reading everything in any community or DM, the record of files sent in calls, and taking things out. Includes `removeContent`. |
| `reviewReports` | The reports people make of messages, profiles, and nicknames, and warning the people reported. |
| `removeContent` | Deleting a reported message, clearing a reported nickname, resetting a reported profile, and deleting a banned account's recent messages. |
| `banUsers` | Banning accounts from the whole deployment. |
| `messageAnyUser` | Messaging anyone, past blocks. |

`aspen-chat-server admin allow <permission>` lets the top role do each of them too.

From then on, administrators manage everything else from the Administration Dashboard. Start
with its Settings tab: the display name and icon the sign-in screen welcomes people with, and the
deployment's policies.

## Making the deployment invite-only

A deployment open to anyone may skip this section. To make it invite-only before anyone has an
account, turn that on from the terminal, then make the first invite:

```
aspen-chat-server settings set --registration-invite-required true
aspen-chat-server invites create
```

The rest of the deployment settings can be set the same way (see
[Deployment settings](../configuration/deployment-settings.md#deployment-settings)).

## Running commands

Commands like these read the same `aspen.toml` as the server. Run them from the same directory,
with the same environment.

---

Previous: [Step 6: Voice servers](6-voice-servers.md) · Next:
[Step 8: Checking it works](8-checking.md)
