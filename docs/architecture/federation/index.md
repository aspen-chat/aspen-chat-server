# Federation

A user of one deployment may use others. Their home deployment is where the account was made; any other is foreign to them, and they are a foreign user there.

Five phases build it, and all five are built:

1. Each deployment's identity, key, policy, and directory of other deployments.
2. Signing in abroad, with foreign users as local rows naming their home.
3. A client holding sessions on several deployments.
4. DMs across deployments and the notices between them.
5. Revocation, deletion notices, and moderating foreign users.

## Pages

- [Identity and keys](identity-and-keys.md): the domain, the key pair, the `/.well-known/aspen` document, and replacing the key.
- [Gates and lists](gates-and-lists.md): emigration and immigration gates, the twelve lists, and how admission is decided.
- [The directory of other deployments](directory.md): `federated_deployment`, pinning keys, suspension, forgetting, and outbound calls.
- [Signing in abroad](signing-in-abroad.md): assertions, foreign users, their profiles, and the multi-deployment client (phases two and three).
- [Protocol versions](protocol-versions.md): versions, capabilities, and the rules in `spec/federation.md`.
- [DMs across deployments and notices](dms-and-notices.md): hosting a DM, `dmJoined`, and verifying statements (phase four).
- [Standing, deletion, and bans](standing.md): the standing check, `accountDeleted`, and banning foreign users (phase five).
- [Design notes](design-notes.md): why it works this way.

## Key files

| Part | Code |
| --- | --- |
| Server | `app::federation`, `api::federation` |
| Domains and keys | `app::federation::Domain`, `app::federation::keys` |
| Directory and contact | `app::federation::directory`, `contact`, `app::federation::fetch` |
| Signing in abroad | `app::federation::abroad`, `app::federation::jws` |
| Verifying statements | `app::federation::received` |
| Notices | `app::federation::notices` |
| Standing | `app::federation::standing` |
| Protocol | `app::federation::protocol` |
| Client | `Deployments`; see `client/docs/architecture/deployments.md` |
| Protocol contract | `spec/federation.md` |
