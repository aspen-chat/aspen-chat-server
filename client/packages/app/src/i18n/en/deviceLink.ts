export const deviceLink = {
  heading: "Other devices",
  sectionHint:
    "Sign in on your phone without typing your password: show a code here and scan it with Aspen on the phone.",
  sectionHintPhone:
    "Sign in on a computer without typing your password: show a code there and scan it here.",
  request: "Sign in with your phone",
  offer: "Sign in on your phone",
  scan: "Scan a sign-in code",
  scanHintSignedIn:
    "Point the camera at the code on the computer you want to sign in. Only scan a code you made yourself, on a device in front of you.",
  scanHintSignedOut:
    "On a computer signed in to Aspen, open Sign-in and security, choose Sign in on your phone, and point the camera at the code.",
  notSignInCode: "That's an invite, not a sign-in code. Open it from Join a community instead.",
  requestHint:
    "On a phone signed in to Aspen, open Sign-in and security, choose Scan a sign-in code, and point the camera here.",
  offerHint:
    "On your phone, open Aspen, choose Scan a sign-in code, and point the camera here. You'll confirm the phone here before it's signed in.",
  codeIsSecret:
    "This code is a secret, like a password. Don't show it to anyone, or let anyone photograph it.",
  codeLabel: "Sign-in code",
  making: "Making a sign-in code…",
  expiresIn: "Expires in {seconds} s",
  expired: "This code expired.",
  newCode: "New code",
  confirmOnPhone: "{device} scanned the code. Confirm on it to finish signing in.",
  confirmOffer: "{device} scanned your code and wants to sign in to your account.",
  confirmOfferWarning:
    "Only sign it in if it's your own phone, scanning your code right now. If you don't recognize it, choose Don't sign in.",
  approvedOffer: "{device} is signed in.",
  confirmRequest: "Sign in {device} to your account?",
  confirmRequestWarning:
    "Only confirm if you just made this code yourself, on a device in front of you. Someone who gets you to scan their code gets into your account.",
  approve: "Sign it in",
  decline: "Don't sign in",
  approvedRequest: "{device} is signed in to your account.",
  waitingForComputer: "Signing in as {account}. Confirm this phone on your computer to finish.",
  cancel: "Cancel",
  cancelled: "Signing in was cancelled. Make a new code on your computer to try again.",
  notVerified: "Signing in another device needs you to confirm it's you first. Try again, and confirm.",
  needsSignedInPhone:
    "This code asks for a sign-in, so it needs a phone that's already signed in. Sign in here first, or scan it with one that is.",
  switchServerPrompt: "This code signs in to {server}. This app uses {here}. Switch to {server}?",
  switchServerWarning:
    "Only switch if you made this code yourself, on your own computer, just now. A code from anyone else could lead you to a server that only looks like yours.",
  switchServer: "Switch to {server}",
  offeredServer:
    "A sign-in code names {server}. Check that this is your deployment's address before you continue.",
  wrongServer:
    "This code is for {server}, but this app uses {here}. Scan it with an app signed in to {server}.",
  incomplete:
    "This sign-in link is incomplete. Scan the code again, or make a new one on the other device.",
  reading: "Reading the sign-in code…",
  signedInHere: "You're signed in.",
  screenHeading: "Sign-in code",
  done: "Done",
  back: "Back",
  deviceName: "{app} on {system}",
} as const;
