# Protocol versions

How the protocol evolves is `spec/federation.md`, the contract any implementation or fork keeps. This page summarises it and says where the code keeps it.

## Where it lives

| Part | Code |
| --- | --- |
| What a deployment speaks | `app::federation::protocol` (`Protocol`, `Software`, `Protocol::with_plugins`) |
| Each peer's, recorded | `federated_deployment.protocol_version`, `protocol_minimum`, `capabilities`, `software_name`, `software_version` |
| Fixtures | `spec/fixtures/federation/` |
| Running two builds together | `scripts/dev_federation.py up --alpha-bin … --beta-bin …` |

## Rules

- Within a protocol version, the protocol changes only by addition.
- Readers ignore unknown fields, enum values, and event, statement, and notice types.
- An unknown enum value is an explicit `Unknown` variant, treated as the most cautious known value. An unknown `Gate` admits no one. A key of an unknown `KeyAlgorithm` is never used.
- Anything else is a new version, served beside the old.
- Every deployment supports the versions of the last thirty-six months.

## What a deployment says it speaks

`Protocol` holds `version`, `minimum`, and `capabilities`. `Software` is for people to read and never decides anything. Both are in the deployment's document and in `GET /auth/methods`.

- Each peer's is recorded on contact, and shown in the dashboard and the terminal.
- A home whose versions do not meet this deployment's is refused (`federationRefused`).
- A client likewise refuses to sign in at a deployment it shares no version with.

## Capabilities

A feature added after a version is released is a capability.

- Bare names are Aspen's.
- A fork names its own by a domain it controls, reversed.
- Every feature is the baseline of version 1, and Aspen's own capability list is empty. Aspen is not yet used outside its developers' machines.
- Each plugin a deployment runs is named among its capabilities by the plugin's id (`Protocol::with_plugins`; see [Plugins](../plugins/index.md)).

## Testing

- `spec/fixtures/federation/` holds payloads of each version and of an imagined newer deployment, which the tests read.
- `scripts/dev_federation.py up --alpha-bin … --beta-bin …` runs two builds against each other.
