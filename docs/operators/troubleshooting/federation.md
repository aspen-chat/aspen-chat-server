# Federation

See also [Federation](../federation/index.md) for setting it up.

## `deploymentUnreachable`

**Means:** this deployment could not read another's document. The detail says why:

- its name has no address;
- it answered only on private addresses;
- it refused the connection;
- it timed out;
- its certificate is expired, not yet valid, for another name, or not from a public authority;
- it redirected;
- it served something that is not an Aspen document.

**What to do:** each detail says whose it is to fix.

Some people are told only that the deployment could not be reached: someone signing in from
another deployment, or a deployment sending a notice. To find the reason:

- look in this server's log, or
- contact the deployment from the dashboard's Federation tab, which shows it too.

To check by hand:

```
curl -v https://<domain>/.well-known/aspen
```

## `federationRefused`

**Means:** this crossing is not allowed. The detail says which of these applies:

- a gate does not let it happen: yours or theirs, for this deployment, for this kind of account;
- federation is off (an `http` `public_url`);
- the two deployments speak no protocol version in common;
- the person is banned here;
- `[federation] max_arrivals_per_home_per_day` people from that deployment already arrived
  today.

**What to do:** if you mean to allow it, change the gate or a list (see
[Gates](../federation/gates.md)).

## `assertionInvalid`

**Means:** a statement from another deployment was refused. The detail says which of these
applies:

- its key changed without a handover. Until its new key is accepted, everything from it is
  refused, even what its old key signs;
- the signature does not match;
- it was meant for another deployment;
- it expired, or is from the future (a clock is wrong);
- it was used before.

**What to do:**

- For a changed key, see [Keys](../federation/keys.md).
- For clocks, run NTP on every server.

## `strongerSignInRequired`

**Means:** this deployment requires two factors, and the visitor signed in at home with a
password alone.

**What to do:** they sign in at home with a second factor or a passkey.
