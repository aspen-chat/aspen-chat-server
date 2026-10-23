# A new release of the web client

A web client page stays open for days, and the deployment may release a new web client
meanwhile. The page keeps working until it loads a part of itself it did not need before (a
language's code highlighting, the QR code reader), which the server may no longer have.

## Where it lives

| Part                       | Code                                                                                                                                            |
| -------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| Naming each build          | `webClientBuild` in `packages/app/vite.config.ts`: `__ASPEN_WEB_CLIENT_BUILD__` in the code, `<meta name="aspen-web-client-build">` in the page |
| What the deployment serves | `webClientBuild` in the event stream's `ready` frame; `AspenSync.webClientBuild`                                                                |
| The strip, and reloading   | `NewRelease` and `releaseReload.ts` in `packages/app/src/features/layout/`                                                                      |
| A part that fails to load  | the `vite:preloadError` listener in `packages/app/src/main.tsx`                                                                                 |

## What happens

- The home deployment's event stream says, at each connection, which build it serves.
- A page whose own build differs is out of date (`isOutdated`). `NewRelease` then shows a strip
  above the app offering to reload, and reloads by itself the next time the page is hidden.
- A part that fails to load (`vite:preloadError`) reloads at once.
- `reloadForNewRelease` reloads at most once a minute in one tab, so a release whose files cannot
  be loaded either shows its error rather than reloading without end.
- Drafts survive the reload: the message box saves its draft when the page is hidden, as a
  reload hides it.

Only a web client the deployment served compares builds. The desktop and mobile apps carry
their own, and the development server's page names none, so neither is ever out of date this
way. The other deployments a client is signed in to are not asked: their web clients are not
this page.
