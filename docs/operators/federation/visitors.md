# Visitors

## Signing in elsewhere

Someone whose home lets them emigrate adds another server either:

- from the "Create or join a community" dialog, or
- by opening an invite link naming it (`/invite/<code>?at=<domain>`).

Their home signs a two-minute statement of who they are for that deployment, which signs them in
there.

- Their profile stays their home's.
- Their status is theirs to set anywhere.

## Usernames

A visitor's username must follow the same rules as one made here. Someone whose name does not
cannot sign in here for the first time until they change it at home. Names that do not follow
them include one:

- holding `@`, spaces, or invisible characters;
- looking like the deployment's own `system` account.

A visitor who later changes to such a name keeps the name they had here.

## How visitors appear

Visitors appear in lists as `name@domain`.

## Banning

Moderators with Ban users can ban a visitor from your deployment in the dashboard's user
directory, as they can your own users. The visitor's sessions end, and they cannot sign in here
again until the ban ends or is lifted.

Banning one of your own users also tells every deployment they visit, at its next standing check,
that they are no longer in good standing there.

## Standing checks

About every `standing_interval_seconds`, this deployment asks each visitor's home whether they
are still in good standing. These visitors are signed out:

- a visitor whose account was deleted;
- one who left;
- one whose home closed its gate to you;
- the visitors of a home unreached for `standing_grace_seconds`.

Homes are asked sixteen at a time, each given twenty seconds. One that keeps failing is asked
less often, up to once an interval.
