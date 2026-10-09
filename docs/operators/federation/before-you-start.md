# Before you start

Federation needs three things.

## A domain that will not change

The host of an `https` `public_url` (see [`public_url`](../configuration/address.md#the-deployments-address)) is this
deployment's name among deployments, such as `chat.example.org`. Add `:port` if it is not served
on 443.

**A deployment that changes its domain is a stranger to every deployment it federated with.**
Other deployments remember the key they find there.

The first server to start with the domain records it in the database. A server started with
another domain, or with an `http` address, refuses to start.

## HTTPS with a certificate from a public authority

Serve `https://<domain>` with a certificate from a public authority, such as Let's Encrypt. Other
deployments reach yours only there, and:

- follow no redirects;
- refuse self-signed certificates;
- give up after ten seconds.

## `/.well-known/aspen` reachable

This is the document other deployments read: your domain, your key, and your gates. The API
servers serve it, like everything at your address. Check it with:

```
curl https://chat.example.org/.well-known/aspen
```

## Your deployment's key

The first API server to start with a domain makes this deployment's key, and keeps it in the
database. Every API server signs with the same one.

**Back the database up accordingly.** See [Backups](../backups.md).
