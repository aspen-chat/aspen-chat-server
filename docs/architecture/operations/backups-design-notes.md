# Backups: design notes

Why the operator's backup guide ([`docs/operators/backups.md`](../../operators/backups.md)) treats
the database as it does.

## Keys in the database

- **The federation key is stored in the database**, so that every API server signs with the same
  one. It follows that anyone holding a database backup can sign as the deployment, so backups
  are as secret as the key.
- **The verification and password reset codes waiting in Valkey are kept under a key in the
  database (`server_secret`)**, so that a copy of Valkey alone does not give the codes away.
