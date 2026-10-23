# Diagrams

Pictures of how Aspen fits together, drawn in [Mermaid](https://mermaid.js.org) so GitHub and most
Markdown viewers render them in place. Each page pairs its diagrams with the few sentences needed to
read them; how each part works is in [`docs/architecture/`](../architecture/), and how to run a
deployment is in [`docs/operators/`](../operators/README.md).

- [The deployment](deployment.md): the processes a deployment runs, the services they share, and
  what reaches what from outside.
- [Rust crates](crates.md): the Cargo workspace's crates, how they depend on one another, and the
  crates outside it.
- [Client packages](client.md): the pnpm workspace, the shells that wrap the web app, and the
  schemas generated from the server that join the two.
- [Flows](flows.md): what happens across those parts when a message is posted, a call is joined, and
  someone signs in to another deployment.

Like every comment and document here, the diagrams describe the code as it stands: a change that
adds a crate, a dependency between crates, a service, or a NATS stream updates them in the same
commit. `cargo metadata --no-deps` lists the workspace's crates and their dependencies, which is how
[Rust crates](crates.md) is checked.
