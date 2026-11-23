# Releasing the desktop app: design notes

Why `desktop-updates.md` is the way it is.

## A release is a tag that runs all of CI

A tag runs `ci.yml` itself, and `publish-windows` waits on the client's checks, so the installer
published is the one that run built, from the commit its checks passed on, and no second workflow
repeats the Windows build's steps. A release costs a full CI run, which a rare event can afford.

## The version is committed, and the tag is checked against it

The version could instead be injected at build time from the tag. Committing it keeps About Aspen,
the installer, and the feed reading one value that a checkout also shows, and the check stops a tag
that disagrees before an hour of building.

## `releases.json` is the record publishing reads back

The publish job needs to know each track's head to refuse an older version and to decide whether a
stable release also heads beta. Reading the feeds back would mean parsing YAML for the same facts
`releases.json` already holds for the site, so the job reads that, and writes it last.

## The updater writes its own configuration

electron-updater reads its feed and the publishers it trusts from `app-update.yml`, which
electron-builder packages. When that file names no publisher, electron-updater installs updates
without checking their signature, and a custom verifier is not even called. Writing the file from
the code's own constants makes the publishers impossible to leave out of a package. It goes through
`updateConfigPath`, which electron-updater marks as meant for tests; the version is pinned, and a
change there shows in the type check.

## The signature check is Aspen's own

electron-updater's check lets an update through when PowerShell cannot run, or cannot convert to
JSON, treating that as an old Windows. On a machine where PowerShell is blocked, that would install
unverified code. `updateSignature.ts` asks the same question of Windows and refuses on every way of
failing; the path reaches PowerShell through the environment, so no file name can be read as code.

## No blockmaps

A blockmap lets electron-updater download only what changed between versions. R2 charges nothing for
what it serves, so the saving goes to nobody, and a whole download has no differential path to go
wrong.

## The installer's flag is per account

The installer is electron-builder's one-click, per-account kind, which needs no administrator. It
cannot write under `HKLM`, and `HKCU\Software\Policies` is writable only by administrators, so the
flag writes `HKCU\Software\Aspen`, and the administrator's switch stays the machine-wide policy key
Group Policy writes.

## A policy that cannot be read turns updating off

An administrator who turned updating off and a machine where the policy cannot be read look alike
from the app. Updating when the policy might say not to would break the administrator's rule, while
not updating costs only a manual download, so an unreadable policy is taken as off.

## The Flatpak repository is rebuilt from the bucket on each release

A release commit needs the branch's head as its parent, the deltas need the commits before it, and
pruning needs the history, so the job reads the whole repository from the bucket, lets Flatpak's own
commands do the work there, and writes back what changed. Keeping a copy anywhere else would be a
second record that could disagree with what people download. Only `HISTORY` commits are kept, so
what is read stays bounded.

## The release commit is made from the CI bundle, signed only at publishing

`desktop-flatpak` builds without the key, on every run. The publish job commits the bundle's app
into the repository with `flatpak build-commit-from`, which signs it and gives it its parent, so the
key is used in one job only, and the bundle published is made from that signed commit.

## The commit's timestamp is the release commit's

A commit's checksum covers its timestamp. Taking the release commit's makes `upload` produce the
same commit when it runs again from the same published state, so a publish that stopped part way
runs again and finds its bundle already there, rather than one for another commit.
