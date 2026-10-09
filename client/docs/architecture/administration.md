# The Administration Dashboard

The Administration Dashboard is where holders of deployment permissions run the deployment: its figures, fleet, invites, newsletter, directories, reports, moderation log, settings, federation, roles, and plugins.

## Where it lives

| Part | Code |
| --- | --- |
| Dashboard | `/admin/{tab}`, `src/features/admin` |
| The caller's deployment permissions | `RecordStore.deploymentPermissions` (topic `admin`); `useDeploymentPermissions`, `useDeploymentCan`, `useIsAdmin` |
| Moderators' reads | `useAdminRead` |
| Directories | `Directory` |
| Growth charts | `LineChart` |
| Deployment roles | `deploymentRoleRecords.ts` |

## Layout

- `/admin` opens the first tab the caller may see.
- On a wide screen, a rail of tabs sits beside the open tab, with the user bar (`SidebarFooter`) at the rail's foot.
- On a one-pane screen, the tabs are set across the top, with the user bar at the screen's foot.
- Each section on a tab is a plane.

## Who sees it

The dashboard is offered in the community rail to anyone with a deployment permission.

The permissions are read at bootstrap from `GET /users/@me/admin` and kept by `deploymentAccessChanged` events.

## Tabs and the permission each needs

| Tab | Permission | Notes |
| --- | --- | --- |
| Totals and growth | `viewDashboard` | See [Figures](#figures) |
| Fleet | `viewDashboard` | |
| Registration invites | `manageRegistrationInvites` | See [Registration invites](#registration-invites) |
| Email newsletter | `sendNewsletters` | `Newsletter.tsx`; see [Newsletter](#newsletter) |
| Directories | `viewDashboard`, `moderateCommunities`, `banUsers`, `reviewReports`, or `removeContent` | See [Directories](#directories) |
| Reports | `reviewReports` | Described in [Reports](reports.md) |
| Report categories | `manageReportCategories` | Described in [Reports](reports.md) |
| Moderation log | `viewDashboard` | See [Moderation log](#moderation-log) |
| Deployment profile and policies | `manageDeploymentSettings` | See [Deployment settings](#deployment-settings) |
| Federation | `manageFederation` | `Federation.tsx`; see [Federation](#federation) |
| Deployment roles | Everyone; editable with `manageDeploymentRoles` | See [Deployment roles](#deployment-roles) |
| File transfer record | `moderateCommunities` | |
| Plugins | `viewDashboard` or `managePlugins` | Described in [Plugins](plugins/index.md) |

## Reads and refreshing

Moderators' reads are the server's answer of the moment, not cached records. `useAdminRead` holds them where they are shown. It also re-reads the fleet every ten seconds while the page is visible.

## Figures

Figures follow the dataviz rules:

- stat tiles for totals
- tables with aligned figures for the fleet
- every state shown as an icon and a word, never colour alone

### Growth charts

Growth is two single-series line charts (`LineChart`, hand-drawn SVG) under one row of range buttons. Users and communities are charted apart. **Why:** one chart with two scales would invent a relation between them.

Each chart has:

- a 2px line over a 10% wash
- round-number gridlines from zero
- its latest value at the end
- a crosshair and readout on hover and from the arrow keys
- a table view carrying every value

## Newsletter

`Newsletter.tsx` lists the posts, newest first: drafts, and the archive of those sent with how many subscribers each went to.

A draft is a subject and a Markdown body. Its life:

1. Saved.
2. Previewed as the server renders the mail, in a sandboxed frame that runs nothing.
3. Sent as a test to the sender's own verified address.
4. Sent to every subscriber, behind a confirmation. From then on it is fixed.

Drafts can be deleted.

## Directories

The directories share `Directory`, which reads its page again when its `version` changes.

The user and community lists:

- sort from their headings (`aria-sort`)
- page by offset, 15 rows by default, with the page size chosen beside them

### Users directory

- Shows each person's deployment roles and, for those who may change them, a picker.
- Shows a user of another deployment as `name@domain`.
- For a moderator, lists anyone's DMs to open.
- Holders of Ban users ban anyone but the system account and themselves from the deployment, and lift bans (`UserBanControl`, `UserBanDialog`; see [Reports](reports.md)). It filters to those banned.
- With `manageDeploymentSettings`, deletes ownerless bots (see [Bots](bots.md)).

### Communities directory

Opens any community.

## Moderation log

Each entry names what it names from the server's `details`:

- people as chips that open their cards
- communities, channels, DMs, and messages as links while they stand
- deleted ones by name
- only what has no name as its id

## Moderators in communities and DMs

A moderator's access is part of the permission resolver (`CommunityPermissions.moderator`, `MODERATION`, both in the shared vectors). A moderator:

- sees every channel
- may take things away: delete messages, remove attachments and others' reactions, remove members, rename and delete channels and communities
- is told in the sidebar when they are in a community only as a moderator

Moderators read no events of a community or DM they are not in. So `AspenSync` applies their writes there to the cache itself (`#readsEventsOf`).

## Registration invites

- An invite copies as its link, `/register?invite=CODE` under the deployment's web client (`shareUrl`; see [QR codes](qr-codes.md)).
- Each usable invite opens its link and QR code (`QrDialog`). One just made does so at once.

### Dual invites

- The form may name a community, among those where the caller may make invites (`useCommunitiesWhere("createInvites")`), to make a dual invite.
- The list names each dual invite's community. It is shown in red while the invite still makes accounts but its community invite no longer works.
- A community's invite dialog makes dual invites too, for those with Manage registration invites ("Also create an account", `MadeDualInvite`).

### Opening the link

- The link opens the create-account screen with the code filled in. For a dual invite, it says which community it joins (`GET /registration-invites/{code}`).
- `RegisterForm` asks for a code whenever `GET /auth/methods` says the server requires one (`useAuthMethods`).
- Signed in, the same link goes on to a dual invite's community invite screen, or home (`RegisterLanding`). Creating the account lands there too.

## Deployment settings

### Profile

`DeploymentProfile.tsx`: the display name and icon the sign-in screens welcome people with. An emptied name is saved as none.

### Policies

`DeploymentSettings.tsx`:

- registration invites
- second factors
- email: an address to register, a verified one to use the server, and a newsletter. None of these can be turned on while the server sends no mail (`emailAvailable`).
- bots
- community limits
- files in calls

## Federation

`Federation.tsx` shows:

- this deployment's domain, key fingerprint, and gates
- adding a deployment, which is contacted at once
- the directory of known deployments. Each can be checked again, put on the lists the gates read, have its offered key reviewed and accepted, or be forgotten.

## Deployment roles

The deployment roles tab shows to everyone. Holders of `manageDeploymentRoles` edit roles below their own highest role (`deploymentRoleRecords.ts`).

A permission another chosen permission includes shows checked and fixed, naming what includes it. Inclusions come from `inclusions` in `GET /users/@me/admin`, read by `includedBy`.
