export const qr = {
  qrCode: "QR code",
  show: "Show QR code",
  inviteLabel: "QR code for invite {code}",
  download: "Download",
  png: "PNG image",
  svg: "SVG image",
  scanInvite: "Scan an invite",
  scanInviteHint: "Point the camera at an invite's QR code.",
  notAnInvite: "That's a sign-in code, not an invite. Scan it from Sign-in and security instead.",
  notAspen:
    "That QR code isn't one of Aspen's. Point the camera at an Aspen invite or sign-in code.",
  cameraStarting: "Starting the camera…",
  cameraLabel: "Camera view",
  cameraDenied:
    "Aspen isn't allowed to use the camera. Allow it in this device's settings, then try again.",
  cameraUnavailable:
    "No camera could be opened. Check that no other app is using it, then try again.",
  dualLabel: "Also create an account",
  dualHint:
    "For someone with no account here yet: the link creates one on this deployment and joins this community with it.",
  dualUses: "Accounts",
  dualMade: "Invite made",
  dualMadeHint:
    "Share this link. It creates an account on this deployment and joins the community; someone who already has an account just joins.",
  dualCommunity: "Also join a community",
  dualNoCommunity: "No community",
  dualJoins: "Joins {community}",
  dualJoinsGone: "Its invite to {community} was revoked or expired",
  registerJoins: "Your new account will also join {community}.",
  openingInvite: "Opening the invite…",
} as const;
