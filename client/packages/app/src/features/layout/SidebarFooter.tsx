import { BookmarksSimpleIcon, TrayIcon } from "@phosphor-icons/react";
import { Link } from "@tanstack/react-router";
import { useCallSource } from "@/api/calls";
import { HomeScope, SourceScope } from "@/api/deployments";
import { useMe } from "@/api/hooks";
import { Avatar } from "@/features/communities/Avatar";
import { Tooltip } from "@/features/layout/Tooltip";
import { useMessages } from "@/i18n/context";
import { SettingsDialog } from "@/features/settings/SettingsDialog";
import { EditProfileDialog } from "@/features/users/EditProfileDialog";
import { displayNameOf, statusLine } from "@/features/users/profile";
import { CallBar } from "@/features/voice/CallBar";
import { VoiceEndedDialog } from "@/features/voice/VoiceEndedDialog";

const footerButtonClass =
  "tap-target rounded-md p-1.5 text-ink-muted outline-none hover:bg-surface-hover hover:text-ink " +
  "pressed:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50";

/**
 * The foot of every list sidebar, the channel list's and the DM list's alike: the user's call,
 * when they are in one on any deployment, and the signed-in user with their profile and
 * settings controls. The dialog that says why a call ended lives here too, so it shows wherever
 * the user is. The user,
 * their profile, and their settings are their home's, even beside another deployment's lists,
 * beside the ways to their activity and saved messages, which span every deployment.
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
  const m = useMessages();
  const me = useMe();
  return (
    <div className="flex items-center gap-3 border-t border-line py-5 ps-3 pe-5">
      {me !== null && <Avatar name={displayNameOf(me)} iconId={me.icon} size="lg" />}
      <span className="flex min-w-0 flex-1 flex-col">
        <span className="truncate text-base font-medium">
          {me === null ? "…" : displayNameOf(me)}
        </span>
        {me?.status != null && (
          <span className="truncate text-sm text-ink-muted">{statusLine(me.status)}</span>
        )}
      </span>
      <Tooltip text={m.activity.open}>
        <Link to="/activity" aria-label={m.activity.open} className={footerButtonClass}>
          <TrayIcon size={20} aria-hidden="true" />
        </Link>
      </Tooltip>
      <Tooltip text={m.saved.open}>
        <Link to="/saved" aria-label={m.saved.open} className={footerButtonClass}>
          <BookmarksSimpleIcon size={20} aria-hidden="true" />
        </Link>
      </Tooltip>
      {me !== null && <EditProfileDialog user={me} triggerClassName={footerButtonClass} />}
      <SettingsDialog triggerClassName={footerButtonClass} />
    </div>
  );
}
