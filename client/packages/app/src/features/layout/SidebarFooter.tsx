import { useCallSource } from "@/api/calls";
import { HomeScope, SourceScope } from "@/api/deployments";
import { useMe } from "@/api/hooks";
import { SettingsDialog } from "@/features/settings/SettingsDialog";
import { EditProfileDialog } from "@/features/users/EditProfileDialog";
import { PresenceMenu } from "@/features/users/PresenceMenu";
import { CallBar } from "@/features/voice/CallBar";
import { VoiceEndedDialog } from "@/features/voice/VoiceEndedDialog";

const footerButtonClass =
  "tap-target rounded-md p-1.5 text-ink-muted outline-none hover:bg-surface-hover hover:text-ink " +
  "pressed:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50";

/**
 * The foot of every list sidebar, the channel list's and the DM list's alike: the user's call,
 * when they are in one on any deployment, and the signed-in user, with their status, which
 * their picture and name open the menu to choose (`PresenceMenu`), and their profile and
 * settings controls. The dialog that says why a call ended lives here too, so it shows wherever
 * the user is. The user, their profile, and their settings are their home's, even beside
 * another deployment's lists. `groundClassName` is the background it sits on.
 */
export function SidebarFooter({
  groundClassName = "bg-surface-raised",
}: {
  groundClassName?: string;
}) {
  const callSource = useCallSource();
  return (
    <>
      {callSource !== null && (
        // The call is wherever the user joined it, which may not be the deployment shown.
        <SourceScope source={callSource}>
          <CallBar />
          <VoiceEndedDialog />
        </SourceScope>
      )}
      <HomeScope>
        <UserFooter groundClassName={groundClassName} />
      </HomeScope>
    </>
  );
}

function UserFooter({ groundClassName }: { groundClassName: string }) {
  const me = useMe();
  return (
    // As tall as the typing line and message box beside it, so their top rules line up.
    <div className="composer-foot-height flex items-center gap-2 border-t border-line py-3 ps-2 pe-5">
      {me === null ? (
        <span className="flex-1 p-1 text-base font-medium">…</span>
      ) : (
        <PresenceMenu me={me} groundClassName={groundClassName} />
      )}
      {me !== null && <EditProfileDialog user={me} triggerClassName={footerButtonClass} />}
      <SettingsDialog triggerClassName={footerButtonClass} />
    </div>
  );
}
