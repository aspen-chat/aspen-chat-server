import { Capacitor } from "@capacitor/core";

/**
 * A light tap felt in the hand as a long press takes, in the mobile apps, which alone can make
 * one (`@capacitor/haptics`, loaded only there); a browser feels nothing and this does nothing.
 * Never awaited by what calls it: the feeling is a side of the press, not a step of it.
 */
export function feelPress(): void {
  if (!Capacitor.isNativePlatform()) {
    return;
  }
  void import("@capacitor/haptics")
    .then(({ Haptics, ImpactStyle }) => Haptics.impact({ style: ImpactStyle.Medium }))
    .catch(() => undefined);
}
