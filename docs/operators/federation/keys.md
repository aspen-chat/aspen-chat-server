# Keys

## Other deployments' keys

The first time this deployment reads another's document, it remembers that deployment's key.
From then on:

| What it finds | What happens |
| --- | --- |
| The same key | Nothing to do. |
| A new key the old one handed over to | Followed on its own. |
| A new key nothing vouches for | **Refused.** See [below](#a-new-key-nothing-vouches-for). |

### A new key nothing vouches for

The deployment is suspended until an administrator accepts the new key:

- everything from it is refused, even what its old key signs;
- its people signed in here are signed out at once.

**A key that changes unannounced can mean someone else is answering at that domain.** To accept
it:

1. Ask its administrators, by some other way than Aspen, whether they replaced their key.
2. Compare fingerprints.
3. Accept the new key in the dashboard, or with:

   ```
   aspen-chat-server federation accept-key <domain> --fingerprint SHA256:…
   ```

Its people then sign in again.

### How soon a new key is noticed

While any gate is open, this deployment reads again, about every `[federation]
standing_interval_seconds`, the document of each deployment it federates with that is in use:

- one you added, noted, or listed;
- one whose people are here;
- one your people use.

So it notices a replaced key within about that long, even when nothing else contacts that
deployment.

## Replacing your own key

### Planned

```
aspen-chat-server federation rotate-key --planned
```

The old key signs a handover to the new one. Every deployment follows it on its own, as long as
it last saw a key among your sixteen newest from the last ninety days.

### Compromised

Use this when the old key may be in someone else's hands:

```
aspen-chat-server federation rotate-key --compromised
```

It vouches for nothing, so every deployment that knew you refuses the new key until its
administrators accept it.

1. Tell their administrators at once, and give them the new fingerprint (`federation status`
   prints it).
2. Each deployment notices the change at its next check (about every `standing_interval_seconds`,
   an hour by default) or sooner. An administrator who runs
   `aspen-chat-server federation contact <your domain>` notices straight away.
3. From then on it refuses everything signed as you, old key or new, and signs your people out,
   until it accepts the new key.

**Until a deployment notices, whoever holds the old key can sign as you there.**
