import type { AspenSync, AspenSyncOptions } from "@aspen/protocol";
import { callName } from "./callName";
import { NativeCall, nativeCalls } from "./nativeCall";

/**
 * The `createCall` an `AspenSync` takes in a mobile app whose calls the native engine carries:
 * a `NativeCall`, named for the system from the sync's store, which `syncOf` answers once the
 * sync is made. Nothing elsewhere, where `AspenSync` makes its `VoiceCall`.
 */
export function nativeCallFactory(
  syncOf: () => AspenSync | null,
): Pick<AspenSyncOptions, "createCall"> {
  if (!nativeCalls()) {
    return {};
  }
  return {
    createCall: (options) =>
      new NativeCall(options, { nameOf: (channelId) => callName(syncOf(), channelId) }),
  };
}
