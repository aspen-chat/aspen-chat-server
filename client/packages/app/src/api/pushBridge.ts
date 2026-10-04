import { EMPTY_PUSH_STATE, type PushDevice, type PushState } from "@aspen/protocol";
import { registerPlugin } from "@capacitor/core";
import { detectShell } from "@/config";

/**
 * The Aspen app's native side of push (`spec/push.md`), which the notification code that runs
 * while the app is closed shares: which platform, app, and relay this build is, and the
 * `PushState` that code reads to decrypt a push and fetch what it points to.
 */
interface AspenPushPlugin {
  describe(): Promise<{
    platform: PushDevice["platform"];
    app: string;
    environment: PushDevice["environment"];
    /** The relay of whoever published this build, which alone can wake it. */
    relay: string;
  }>;
  loadState(): Promise<{ state: string | null }>;
  saveState(options: { state: string }): Promise<void>;
}

export const AspenPush = registerPlugin<AspenPushPlugin>("AspenPush");

/**
 * What a notification the native code posted carries, for opening its message (or, for a
 * plugin's notice about no message, its channel) when it is tapped: the deployment, and where
 * the message is.
 */
export interface NotificationTarget {
  origin: string;
  channel: string;
  message: string | null;
  community: string | null;
  /** The channel a thread's message is in, when it is in a thread. */
  parentChannel: string | null;
}

export async function loadState(): Promise<PushState> {
  const { state } = await AspenPush.loadState();
  if (state === null) {
    return EMPTY_PUSH_STATE;
  }
  const parsed = JSON.parse(state) as { version?: unknown };
  return parsed.version === 1 ? (parsed as PushState) : EMPTY_PUSH_STATE;
}

/**
 * Forgets every account the phone is woken for, as signing out does; the deployments drop their
 * subscriptions with the sign-in, and the relay's are swept at the next sign-in.
 */
export async function forgetPushAccounts(): Promise<void> {
  if (detectShell() !== "mobile") {
    return;
  }
  const state = await loadState().catch(() => null);
  if (state !== null) {
    await AspenPush.saveState({ state: JSON.stringify({ ...state, accounts: [] }) }).catch(
      () => undefined,
    );
  }
}
