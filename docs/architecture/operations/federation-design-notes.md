# Federation (operators): design notes

Why the operator's federation guide ([`docs/operators/federation/`](../../operators/federation/index.md))
works as it does. The protocol's own design is in [Federation](../federation/index.md).

## Domain and key

- **The domain never changes.** Other deployments remember the key they find at it, so a
  deployment that changes its domain is a stranger to all of them. See
  [Before you start](../../operators/federation/before-you-start.md).
- **The deployment's key is kept in the database**, so every API server signs with the same one.

## Keys

- **A new key nothing vouches for suspends the deployment, and even what its old key signs is
  refused.** The old key may be the one that leaked. See
  [Keys](../../operators/federation/keys.md#a-new-key-nothing-vouches-for).

## Lists

- **A deployment on a block list cannot be forgotten.** Forgetting it would take it off the list
  and let it in. See [The directory](../../operators/federation/directory.md#forgetting-a-deployment).

## Standing checks

- **Homes are asked sixteen at a time, each given twenty seconds, and one that keeps failing is
  asked less often, up to once an interval**, so slow homes hold up no one else. See
  [Visitors](../../operators/federation/visitors.md#standing-checks).
