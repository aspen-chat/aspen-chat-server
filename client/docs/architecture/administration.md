# The Administration Dashboard

- The Administration Dashboard is `/admin/{tab}` (`src/features/admin`; `/admin` opens the first
  tab the caller may), a rail of tabs beside the one open, with the user bar (`SidebarFooter`) at its
  foot, set across the top on a one-pane screen with the user bar at the screen's foot, each section on it a plane; it is offered in the community
  rail to anyone with a deployment permission (`RecordStore.deploymentPermissions`, topic
  `admin`, read at bootstrap from `GET /users/@me/admin` and kept by `deploymentAccessChanged`
  events; `useDeploymentPermissions`, `useDeploymentCan`, `useIsAdmin`). Each tab shows
  only with the permission it needs: the totals and growth, and the fleet, with `viewDashboard`,
  registration invites with `manageRegistrationInvites`, the email newsletter with
  `sendNewsletters` (`Newsletter.tsx`: the posts, newest first, drafts and the archive of those
  sent with how many subscribers each went to; a draft's subject and Markdown body, saved, then
  previewed as the server renders the mail, in a sandboxed frame that runs nothing, sent as a
  test to the sender's own verified address, and sent to every subscriber behind a
  confirmation, fixed from then on; drafts deleted), the directories with `viewDashboard`,
  `moderateCommunities`, `banUsers`, `reviewReports`, or `removeContent`, the reports with `reviewReports`, the
  report categories with `manageReportCategories`, the moderation log with `viewDashboard`, the deployment's profile and policies with `manageDeploymentSettings`
  (`DeploymentProfile.tsx`: the display name and icon the sign-in screens welcome people with,
  an emptied name saved as none; `DeploymentSettings.tsx`: registration invites, second
  factors, email (an address to register, a verified one to use the server, and a newsletter,
  none of which can be turned on while the server sends no mail, `emailAvailable`), bots,
  community limits, and files in calls), federation with
  `manageFederation` (`Federation.tsx`: this deployment's domain, key fingerprint, and gates;
  adding a deployment, which is contacted at once; and the directory of those known, each
  checked again, put on the lists the gates read, its offered key reviewed and accepted, or
  forgotten), and the deployment roles to everyone, editable with `manageDeploymentRoles` below
  the caller's highest role (`deploymentRoleRecords.ts`); a permission another chosen one
  includes (`inclusions` from `GET /users/@me/admin`, read by `includedBy`) shows checked and
  fixed, naming what includes it. The file transfer record shows with `moderateCommunities`. The directories share `Directory`, which
  reads its page again when its `version` changes. The users directory shows each person's deployment roles and, for
  those who may, a picker to change them; for a moderator, it lists anyone's DMs to open, and
  the communities directory opens any community. The moderation log names what each entry names from the server's `details`: people as chips that open their cards, communities, channels, DMs, and messages as links while they stand, deleted ones by name, and only what has no name as its id. A moderator's access is part of the resolver
  (`CommunityPermissions.moderator`, `MODERATION`, both in the shared vectors): they see every
  channel, may take things away (delete messages, remove attachments and others' reactions,
  remove members, rename and delete channels and communities), and are told in the sidebar
  when they are in a community only as a moderator. They read no events of a community or DM
  they are not in, so `AspenSync` applies their writes there to the cache itself
  (`#readsEventsOf`). Their reads are the server's answer of the moment rather than cached
  records, held where they are shown by `useAdminRead`, which also re-reads the fleet every ten
  seconds while the page is visible. Figures follow the dataviz rules: stat tiles for totals,
  tables with aligned figures for the fleet, and every state an icon and a word, never color
  alone. Growth is two single-series line charts (`LineChart`, hand-drawn SVG) under one row of
  range buttons, users and communities apart because one chart with two scales would invent a
  relation between them; each has a 2px line over a 10% wash, round-number gridlines from zero,
  its latest value at the end, a crosshair and readout on hover and from the arrow keys, and the
  table view carries every value. The user and community lists sort from their headings
  (`aria-sort`) and page by offset, 15 rows by default, with the page size chosen beside them. A
  registration invite copies as its link, `/register?invite=CODE` under the deployment's web
  client (`shareUrl`, see QR codes), and each usable one opens its link and QR code
  (`QrDialog`), as one just made does at once. The form may name a community, among those where
  the caller may make invites (`useCommunitiesWhere("createInvites")`), to make a dual invite,
  and the list names each dual invite's community, in red while the invite still makes accounts
  but its community invite no longer works. A community's invite dialog makes dual invites too,
  for those with Manage registration invites ("Also create an account", `MadeDualInvite`). The
  link opens the create-account screen with the code filled in, saying which community a dual
  invite joins (`GET /registration-invites/{code}`), and `RegisterForm` asks for a code whenever
  `GET /auth/methods` says the server requires one (`useAuthMethods`). Signed in, the same link
  (which is also where creating the account lands) goes on to a dual invite's community invite
  screen, or home (`RegisterLanding`).
- The Administration Dashboard's user directory shows a user of another deployment as
  `name@domain`. Holders of Ban users ban anyone but the system account and themselves from the
  deployment, and lift bans, there (`UserBanControl`, `UserBanDialog`; see Reports), and it
  filters to those banned. The Reports and Report categories tabs are described under
  Reports, and the Plugins tab, for `viewDashboard` or `managePlugins`, under Plugins.
