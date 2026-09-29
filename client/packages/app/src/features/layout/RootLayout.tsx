import { Outlet, useLocation, useParams, useSearch } from "@tanstack/react-router";
import { useState } from "react";
import { useAspenClient, useSession } from "@/api/context";
import { useOneCallAtATime } from "@/api/calls";
import { ShareBlocksAcrossDeployments } from "@/api/identity";
import { WakeThisPhone } from "@/api/push";
import { SyncProvider } from "@/api/sync";
import { CommunityRail } from "@/features/communities/CommunityRail";
import { LoginForm } from "@/features/auth/LoginForm";
import { RegisterForm } from "@/features/auth/RegisterForm";
import { useServerChoice } from "@/features/auth/serverChoice";
import { SyncBanner } from "@/features/layout/SyncBanner";
import { EnrollmentScreen } from "@/features/security/EnrollmentScreen";
import { FollowLanguagePreference } from "@/features/settings/LanguageSection";
import { SourcePickerDialog } from "@/features/voice/SourcePickerDialog";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/**
 * Signed out: the sign-in or create-account screen, leaving the URL alone so a shared link
 * opens once the user is in. Signed in to an account that still owes the server a second
 * factor: the screen for adding one. Signed in: the community rail beside whatever the route
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
  return <SignedIn />;
}

/**
 * Signed out. An invite link naming another deployment says the user's account may be on any
 * deployment. A registration link (`/register`, with `?invite=` from the Administration
 * Dashboard) opens on the create-account screen with the invite filled in.
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
  const [screen, setScreen] = useState<"login" | "register">(
    pathname === "/register" || invite !== undefined ? "register" : "login",
  );
  return (
    <main className="flex min-h-full flex-col items-center justify-center gap-4 p-6">
      {inviteAt !== null && (
        <p className="max-w-sm text-center text-sm text-ink-muted">
          {format(m.inviteOnDomain, { domain: inviteAt })}
        </p>
      )}
      {screen === "register" ? (
        <RegisterForm
          initialInvite={invite}
          onSwitchToLogin={() => {
            setScreen("login");
          }}
        />
      ) : (
        <LoginForm
          serverUrl={serverUrl}
          onChangeServer={changeServer}
          onSwitchToRegister={() => {
            setScreen("register");
          }}
        />
      )}
    </main>
  );
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
      <FollowLanguagePreference />
      <div className="flex h-full flex-col">
        <SyncBanner />
        <SourcePickerDialog />
        <div className="flex min-h-0 flex-1">
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
