import { useCallSource } from "@/api/calls";
import { HomeScope, SourceScope } from "@/api/deployments";
import { useMe } from "@/api/hooks";
import { Avatar } from "@/features/communities/Avatar";
import { SettingsDialog } from "@/features/settings/SettingsDialog";
import { EditProfileDialog } from "@/features/users/EditProfileDialog";
import { displayNameOf, statusLine } from "@/features/users/profile";
import { CallBar } from "@/features/voice/CallBar";
import { VoiceEndedDialog } from "@/features/voice/VoiceEndedDialog";

const footerButtonClass =
  "tap-target rounded-md p-1 text-ink-muted outline-none hover:bg-surface-hover hover:text-ink " +
  "pressed:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50";

/**
 * The foot of every list sidebar, the channel list's and the DM list's alike: the user's call,
 * when they are in one on any deployment, and the signed-in user with their profile and
 * settings controls. The dialog that says why a call ended lives here too, so it shows wherever
 * the user is. The user,
 * their profile, and their settings are their home's, even beside another deployment's lists.
 */
export function SidebarFooter() {
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
        <UserFooter />
      </HomeScope>
    </>
  );
}

function UserFooter() {
  const me = useMe();
  return (
    <div className="flex items-center gap-2 border-t border-line px-3 py-2">
      {me !== null && <Avatar name={displayNameOf(me)} iconId={me.icon} size="sm" />}
      <span className="flex min-w-0 flex-1 flex-col">
        <span className="truncate text-sm font-medium">
          {me === null ? "…" : displayNameOf(me)}
        </span>
        {me?.status != null && (
          <span className="truncate text-xs text-ink-muted">{statusLine(me.status)}</span>
        )}
      </span>
      {me !== null && <EditProfileDialog user={me} triggerClassName={footerButtonClass} />}
      <SettingsDialog triggerClassName={footerButtonClass} />
    </div>
  );
}
