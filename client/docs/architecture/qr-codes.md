# QR codes

- Every QR code the app shows is drawn by `QrCode` (`src/features/qr/QrCode.tsx`) from
  `qrDrawing` (`qrDrawing.ts`): the `qr` package's modules at error correction `high`, with a
  four-module quiet zone, black on white whatever the theme, and Aspen's line mark in the
  middle (`brand/aspen-mark-line.svg`, drawn by `scripts/export_icons.py` from the canopy's own
  circles and trunk, as black strokes with no fill or bark). The modules under the middle patch
  (about a quarter of the width) are left light rather than painted over, and error correction
  restores them; `qrDrawing.test.ts` decodes codes of the lengths the app makes with the patch
  empty, so a change that breaks scanning fails there. `cover` blurs a code and lays something
  over it, as an expired sign-in code does.
- `QrDownload` saves a code as the reader picks: an SVG (`qrSvgDocument`, the same drawing as a
  standalone document) or a 1024-pixel PNG rendered from it. Saving goes through `saveFile`
  (`src/features/layout/saveFile.ts`): a download in browsers and Electron, and the system's
  picker in the mobile apps (`filesBridge`), whose web views cannot follow a download link.
- Invites show their code wherever they are made or looked at (`InviteQr`): a community's
  invite dialog opens the code of an invite just made and of any other on request, the
  dashboard's registration invites open theirs in a dialog, and a dual invite just made shows
  its registration link's.
- The links codes hold, and every invite link copied, are made by `shareUrl`
  (`src/features/qr/shareLinks.ts`) under the home deployment's web client, which `GET
/deployment` names (`webClientUrl`, its `public_url`), so they open on any device;
  where that cannot be read, under the web client serving the page, and in the desktop and mobile
  apps as `aspen://app/…`, which only an installed app opens. `useShareUrl` is `null` until the
  deployment has said, so no link or code shows an address that is about to change.
- Phones scan with `QrScannerDialog` (`QrScanner.tsx`): the back camera through `getUserMedia`,
  frames scaled to 640 pixels across read five times a second by `zxing-wasm`'s reader, whose
  WebAssembly is served with the app (`zxing.ts`) and loaded when the scanner first opens.
  `ScanCodeButton` reads what a code leads to (`parseAspenLink`, `aspenLinks.ts`: a sign-in
  code, a registration invite, a community invite, or a channel or DM, as the links in the
  server's mail name them (`/communities/{id}/channels/{id}`, `/dms/{id}`), under whatever
  address it was shared) and answers a code that is not Aspen's, or not the kind wanted there,
  without closing; the
  join form scans invites, and Sign-in and security, the sign-in screen, and the server form
  scan sign-in codes. In the desktop and mobile apps `useOpenAppLinks` (`src/api/appLinks.ts`) opens
  the `aspen://app/…` links the system hands over, at launch or while running, the same way.
  The desktop installers register the scheme (`protocols` in `electron-builder.yml`), a
  packaged app claims it again at run time (a development build does not, which would make a
  bare Electron the system's handler), and one instance runs at a time: a link opened while it
  runs comes as the second launch's command line on Windows and Linux, or `open-url` on macOS,
  and the main process (`packages/desktop/src/main/appLinks.ts`) passes only `aspen://app/…`
  links to the page, holding them until it asks (`appLinks` on the preload bridge). A link
  arriving while signed out opens on its own screen, a registration link on the create-account
  form with its code.
