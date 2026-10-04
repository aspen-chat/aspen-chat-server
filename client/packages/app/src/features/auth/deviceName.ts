import { detectShell, type Shell } from "@/config";

/** The operating system a user agent names, or `null` when it names none Aspen runs on. */
export function systemOf(userAgent: string): string | null {
  if (userAgent.includes("Android")) {
    return "Android";
  }
  if (userAgent.includes("iPad")) {
    return "iPad";
  }
  if (/iPhone|iPod/.test(userAgent)) {
    return "iPhone";
  }
  if (userAgent.includes("CrOS")) {
    return "ChromeOS";
  }
  if (/Mac OS X|Macintosh/.test(userAgent)) {
    return "macOS";
  }
  if (userAgent.includes("Windows")) {
    return "Windows";
  }
  if (userAgent.includes("Linux")) {
    return "Linux";
  }
  return null;
}

/** The browser a user agent names, checked in an order that tells the Chromium family apart. */
export function browserOf(userAgent: string): string | null {
  if (userAgent.includes("Edg/")) {
    return "Edge";
  }
  if (userAgent.includes("OPR/")) {
    return "Opera";
  }
  if (/Firefox\/|FxiOS\//.test(userAgent)) {
    return "Firefox";
  }
  if (/Chrome\/|CriOS\//.test(userAgent)) {
    return "Chrome";
  }
  if (userAgent.includes("Safari/")) {
    return "Safari";
  }
  return null;
}

/**
 * What this device calls itself to the one that confirms a sign-in code: the app or browser
 * and the system it runs on ("Firefox on Linux", "Aspen on Android"), which the person
 * confirming recognizes as the device in front of them. `template` puts the two together in the
 * reader's language. It names no place and nothing a network lookup would tell.
 */
export function thisDeviceName(
  template: (app: string, system: string | null) => string,
  userAgent: string = navigator.userAgent,
  shell: Shell = detectShell(),
): string {
  const app = shell === "web" ? (browserOf(userAgent) ?? "Aspen") : "Aspen";
  return template(app, systemOf(userAgent));
}
