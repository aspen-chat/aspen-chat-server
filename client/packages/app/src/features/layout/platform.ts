import { Capacitor } from "@capacitor/core";
import { isAppleTouch } from "@/theme/systemTextSize";

/**
 * Whether this is a phone or tablet: the mobile apps, or a browser on Android, iOS, or iPadOS.
 * It is the platform, not the screen's width, so a narrow window on a computer keeps what a
 * computer offers.
 */
export function isMobilePlatform(): boolean {
  if (Capacitor.isNativePlatform()) {
    return true;
  }
  if (typeof navigator === "undefined") {
    return false;
  }
  return (
    navigator.userAgent.includes("Android") ||
    isAppleTouch(navigator.userAgent, navigator.maxTouchPoints)
  );
}
