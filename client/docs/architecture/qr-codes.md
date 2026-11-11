# QR codes

## Where it lives

| Part           | Where                                                                              |
| -------------- | ---------------------------------------------------------------------------------- |
| Drawing a code | `QrCode` (`src/features/qr/QrCode.tsx`), `qrDrawing` (`qrDrawing.ts`)              |
| Saving a code  | `QrDownload`, `qrSvgDocument`, `saveFile` (`src/features/layout/saveFile.ts`)      |
| Invite codes   | `InviteQr`                                                                         |
| Links in codes | `shareUrl`, `useShareUrl` (`src/features/qr/shareLinks.ts`)                        |
| Scanning       | `QrScannerDialog` (`QrScanner.tsx`), `zxing.ts`, `ScanCodeButton`                  |
| Reading a link | `parseAspenLink` (`aspenLinks.ts`), `routeAddress` (`routeAddress.ts`)             |
| App links      | `useOpenAppLinks` (`src/api/appLinks.ts`), `packages/desktop/src/main/appLinks.ts` |

## Drawing

Every QR code the app shows is drawn by `QrCode` from `qrDrawing`:

- the `qr` package's modules at error correction `high`;
- a four-module quiet zone;
- black on white, whatever the theme;
- Aspen's line mark in the middle (`brand/aspen-mark-line.svg`).

`scripts/export_icons.py` draws the line mark from the canopy's own circles and trunk, as black
strokes with no fill or bark.

The modules under the middle patch (about a quarter of the width) are left light rather than painted
over. Error correction restores them.

**`qrDrawing.test.ts` decodes codes of the lengths the app makes with the patch empty.** A change
that breaks scanning fails there.

`cover` blurs a code and lays something over it, as an expired sign-in code does.

## Saving

`QrDownload` saves a code as the reader picks:

- an SVG (`qrSvgDocument`, the same drawing as a standalone document);
- a 1024-pixel PNG rendered from it.

Saving goes through `saveFile`:

| Where                 | How                                                                                |
| --------------------- | ---------------------------------------------------------------------------------- |
| Browsers and Electron | A download                                                                         |
| Mobile apps           | The system's picker (`filesBridge`); their web views cannot follow a download link |

## Invite codes

Invites show their code wherever they are made or looked at (`InviteQr`):

- A community's invite dialog opens the code of an invite just made, and of any other on request.
- The dashboard's registration invites open theirs in a dialog.
- A dual invite just made shows its registration link's code.

## Links in codes

`shareUrl` (`src/features/qr/shareLinks.ts`) makes the links codes hold, and every invite link
copied.

| Case                                              | Links are under                                                                                               |
| ------------------------------------------------- | ------------------------------------------------------------------------------------------------------------- |
| The home deployment's web client is known         | That web client, which `GET /deployment` names (`webClientUrl`, its `public_url`), so they open on any device |
| It cannot be read, in a browser                   | The web client serving the page                                                                               |
| It cannot be read, in the desktop and mobile apps | `aspen://app/…`, which only an installed app opens                                                            |

`useShareUrl` is `null` until the deployment has said, so no link or code shows an address that is
about to change.

## Scanning

Phones scan with `QrScannerDialog`:

- the back camera, through `getUserMedia`;
- frames scaled to 640 pixels across;
- read five times a second by `zxing-wasm`'s reader;
- its WebAssembly is served with the app (`zxing.ts`) and loaded when the scanner first opens.

`ScanCodeButton` reads what a code leads to with `parseAspenLink`:

- a sign-in code;
- a registration invite;
- a community invite;
- a channel or DM, as the links in the server's mail name them (`/communities/{id}/channels/{id}`,
  `/dms/{id}`).

It answers a code that is not Aspen's, or not the kind wanted there, without closing.

### Reading a link

`routeAddress` reads the text as one address, in each form a link to the app takes:

| Form                         | Example                                                     | Its host                            |
| ---------------------------- | ----------------------------------------------------------- | ----------------------------------- |
| Under a web client           | `https://chat.example.org/invite/abc`                       | The address's                       |
| An app link                  | `aspen://app/invite/abc`                                    | None                                |
| An app's page, route in hash | `file:///…/index.html#/invite/abc`, `https://localhost/#/…` | None for a file, else the address's |

- A route is the link's whole path. A route found in a query, in a longer path, in a fragment that
  is not a page's route, or in text around a link is not read.
- What a link leads to is the deployment's whose address it is on. The home's own addresses
  (`useHomeHosts`: the page's, and the one the app reaches the home at) and links with no host mean
  the deployment in use. Any other host is that deployment's domain (`deploymentRoute`).
- `/at/{domain}/…` names the deployment, whatever address it is under.
- A sign-in code names its server itself.
- A registration invite on another deployment's address is what that deployment asks of a first
  arrival, not a way to make an account there, for a reader who has one at home. The join form,
  whether it was scanned or pasted, opens the add-a-server form with the deployment and the invite
  filled in (`routeJoinLink`, `features/invites/joinRouting.ts`; see
  [Adding, leaving, and signing out](deployments.md#adding-leaving-and-signing-out)). Where the home
  takes no part in federation it is refused, naming that deployment.
- Signed out, no screen reads another deployment's registration invite: the scanners there take
  sign-in codes only, and a registration link opened in an app (`aspen://app/register?invite=…`) or
  under the web client is the deployment in use's, and opens its create-account form with the code.

| Where                | Scans         |
| -------------------- | ------------- |
| The join form        | Invites       |
| Sign-in and security | Sign-in codes |
| The sign-in screen   | Sign-in codes |
| The server form      | Sign-in codes |

## App links

In the desktop and mobile apps, `useOpenAppLinks` opens the `aspen://app/…` links the system hands
over, at launch or while running, the same way a scan does.

On the desktop:

- The installers register the scheme (`protocols` in `electron-builder.yml`).
- A packaged app claims it again at run time. A development build does not, which would make a bare
  Electron the system's handler.
- One instance runs at a time. A link opened while it runs comes as:
  - the second launch's command line, on Windows and Linux;
  - `open-url`, on macOS.
- The main process (`packages/desktop/src/main/appLinks.ts`) passes only `aspen://app/…` links to
  the page. It holds them until the page asks (`appLinks` on the preload bridge).

A link arriving while signed out opens on its own screen. A registration link opens on the
create-account form with its code.
