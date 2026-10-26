import { Capacitor, registerPlugin } from "@capacitor/core";

/**
 * The mobile apps' native side of the age check (`AspenAgePlugin`, in each): whether the device
 * says its user is under 18 where the law asks apps to check, through Declared Age Range on iOS
 * and Play Age Signals on Android. The answer is never stored, here or anywhere else.
 */
interface AspenAgePlugin {
  check(): Promise<{ underage: boolean }>;
}

const AspenAge = registerPlugin<AspenAgePlugin>("AspenAge");

/** Whether this app can ask its device for its user's age: only the mobile apps can. */
export function canAskAge(): boolean {
  return Capacitor.isNativePlatform() && Capacitor.isPluginAvailable("AspenAge");
}

/** The one answer of this launch, so the device is asked once however many times it is wanted. */
let asked: Promise<boolean> | null = null;

/**
 * Whether the device says its user is too young for Aspen, asked once each time the app starts.
 * Wherever the device reports no age, or the check fails, the answer is `false`.
 */
export function deviceReportsUnderage(): Promise<boolean> {
  asked ??= AspenAge.check().then(
    (answer) => answer.underage,
    () => false,
  );
  return asked;
}
