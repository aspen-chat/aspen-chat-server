import { ReleaseStrip } from "@/features/layout/ReleaseStrip";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";
import { useDesktopUpdates } from "./useDesktopUpdates";

/**
 * The strip above the app once the desktop app has downloaded an update, which says it installs
 * when the app quits and offers to restart into it now.
 */
export function UpdateReady() {
  const m = useMessages();
  const state = useDesktopUpdates();
  if (state?.status !== "ready") {
    return null;
  }
  return (
    <ReleaseStrip
      text={format(m.updates.ready, { version: state.version ?? "" })}
      action={m.updates.restart}
      onAction={() => {
        window.aspenDesktop?.updates.restart();
      }}
    />
  );
}
