import { Outlet, useLocation, useParams, useSearch } from "@tanstack/react-router";
import { Announcer } from "@/features/layout/Announcer";
import { useFollowMessageTextSize } from "@/features/layout/messageTextSize";
import { useFollowMotionSpeed } from "@/features/layout/motion";
import { useState } from "react";
import { useAspenClient, useSession } from "@/api/context";
import { useOneCallAtATime } from "@/api/calls";
import { ShareBlocksAcrossDeployments } from "@/api/identity";
import { WakeThisPhone } from "@/api/push";
import { NotifyOnMessages } from "@/features/notifications/NotifyOnMessages";
import { SyncProvider } from "@/api/sync";
import { CommunityRail } from "@/features/communities/CommunityRail";
import { DeploymentWelcome } from "@/features/auth/DeploymentWelcome";
import { DeviceLinkRoute } from "@/features/auth/DeviceLinkScreen";
import { LoginForm } from "@/features/auth/LoginForm";
import { RegisterForm } from "@/features/auth/RegisterForm";
import { useServerChoice } from "@/features/auth/serverChoice";
import { SyncBanner } from "@/features/layout/SyncBanner";
import { EnrollmentScreen } from "@/features/security/EnrollmentScreen";
import { VerificationScreen } from "@/features/email/VerificationScreen";
import { FollowLanguagePreference } from "@/features/settings/LanguageSection";
import { SourcePickerDialog } from "@/features/voice/SourcePickerDialog";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";
import { IncomingCalls } from "@/features/voice/IncomingCall";

/**
 * Signed out: the sign-in or create-account screen, leaving the URL alone so a shared link
 * opens once the user is in. Signed in to an account that still owes the server a second
 * factor: the screen for adding one; or a verified email address: the screen for verifying it. Signed in: the community rail beside whatever the route
 * shows, all fed by one `SyncProvider`.
 */
export function RootLayout() {
  const session = useSession();
  if (session === null) {
    return <SignedOut />;
  }
  if (session.twoFactorEnrollmentRequired === true) {
    return <EnrollmentScreen />;
  }
  if (session.emailVerificationRequired === true) {
    return <VerificationScreen />;
  }
  return <SignedIn />;
}

/**
 * Signed out: the deployment's welcome above the sign-in or create-account form. An invite link
 * naming another deployment says the user's account may be on any deployment. A registration link (`/register`, with `?invite=` from the Administration
 * Dashboard) opens on the create-account screen with the invite filled in. A sign-in code
 * (`/device-link`) opens its own screen instead.
 */
function SignedOut() {
  const { serverUrl, changeServer } = useServerChoice();
  const { pathname } = useLocation();
  const m = useMessages();
  const search: { invite?: unknown; at?: unknown } = useSearch({ strict: false });
  // An invite link naming its deployment opens once the user is in, wherever their account is.
  const inviteAt =
    pathname.startsWith("/invite/") && typeof search.at === "string" ? search.at : null;
  const invite = typeof search.invite === "string" ? search.invite : undefined;
  // The screen a link opens on, until the reader switches; a link opened later (one the desktop
  // or mobile app is handed while it runs) opens on its own screen again.
  const opened = `${pathname}?${invite ?? ""}`;
  const [switched, setSwitched] = useState<{ on: string; screen: "login" | "register" } | null>(
    null,
  );
  const screen =
    switched?.on === opened
      ? switched.screen
      : pathname === "/register" || invite !== undefined
        ? "register"
        : "login";
  const setScreen = (next: "login" | "register") => {
    setSwitched({ on: opened, screen: next });
  };
  // A sign-in code scanned or opened here signs this device in when the other one confirms it.
  if (pathname === "/device-link") {
    return (
      <main className="flex min-h-full flex-col items-center justify-center">
        <DeviceLinkRoute />
      </main>
    );
  }
  return (
    <main className="flex min-h-full flex-col items-center justify-center gap-4 p-6">
      {inviteAt !== null && (
        <p className="max-w-sm text-center text-sm text-ink-muted">
          {format(m.inviteOnDomain, { domain: inviteAt })}
        </p>
      )}
      <DeploymentWelcome serverUrl={serverUrl} onChangeServer={changeServer} />
      {screen === "register" ? (
        <RegisterForm
          // A new link's code fills the form afresh.
          key={invite ?? ""}
          initialInvite={invite}
          onSwitchToLogin={() => {
            setScreen("login");
          }}
        />
      ) : (
        <LoginForm
          onSwitchToRegister={() => {
            setScreen("register");
          }}
        />
      )}
    </main>
  );
}

/** Keeps the page moving at the reader's animation speed. */
function FollowMotionSpeed() {
  useFollowMotionSpeed();
  return null;
}

/** Keeps messages drawn at the reader's message text size. */
function FollowMessageTextSize() {
  useFollowMessageTextSize();
  return null;
}

/**
 * The signed-in app: the community rail beside the route. On a narrow screen a conversation
 * (a channel, a DM, a thread) takes the whole width, and the rail shows with the lists the
 * back links lead to.
 */
function SignedIn() {
  const client = useAspenClient();
  const { channelId } = useParams({ strict: false });
  return (
    <SyncProvider client={client}>
      <OneCall />
      <ShareBlocksAcrossDeployments />
      <WakeThisPhone />
      <NotifyOnMessages />
      <IncomingCalls />
      <FollowLanguagePreference />
      <FollowMotionSpeed />
      <FollowMessageTextSize />
      <Announcer />
      <div className="flex h-full flex-col">
        <SyncBanner />
        <SourcePickerDialog />
        {/* Panels slide in from beyond the screen's edge; nothing there may widen the page. */}
        <div className="flex min-h-0 flex-1 overflow-x-clip">
          <div className={channelId === undefined ? "flex" : "hidden md:flex"}>
            <CommunityRail />
          </div>
          <Outlet />
        </div>
      </div>
    </SyncProvider>
  );
}

/** Keeps the user in one call across every deployment they use. */
function OneCall() {
  useOneCallAtATime();
  return null;
}
