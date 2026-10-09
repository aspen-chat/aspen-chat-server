# Federation

Federation lets the people of your deployment use other Aspen deployments with the account they
already have, and other deployments' people use yours.

- Someone's **home** is the deployment where they made their account.
- Any other deployment they use is **foreign** to them.
- Signing in at home is all they ever do: their home vouches for them to the others.

Nothing crosses until you say so. Every gate is closed by default.

## Pages

- [Before you start](before-you-start.md): the domain, certificate, and document federation needs.
- [Gates](gates.md): who may cross, each way, and the usual policies.
- [The directory](directory.md): the deployments this one knows, and forgetting them.
- [Keys](keys.md): other deployments' keys, and replacing your own.
- [Visitors](visitors.md): what people see, banning visitors, and standing checks.
- [Protocol versions](protocol-versions.md): staying compatible with other deployments.

## Trying it on one machine

`scripts/dev_federation.py up` runs two deployments, `alpha.localhost` and `beta.localhost`,
with a development certificate authority. `scripts/dev_federation.py check` exercises federation
between them.
