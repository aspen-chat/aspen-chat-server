import { Outlet, useLocation, useParams, useSearch } from "@tanstack/react-router";
import { Announcer } from "@/features/layout/Announcer";
import { useFollowMessageTextSize } from "@/features/layout/messageTextSize";
import { useFollowMotionSpeed } from "@/features/layout/motion";
import { useContext, useEffect, useMemo, useState, type ReactNode } from "react";
import {
  AccountContext,
  useAccount,
  useAccounts,
  useAccountsHub,
  useAllSources,
  type AccountEntry,
} from "@/api/accounts";
import { AspenClientContext, HomeClientContext, useAspenClient, useSession } from "@/api/context";
import { useEndSignIns } from "@/api/deploymentsContext";
import { useSources } from "@/api/everywhere";
import { useFollowSpeakerphone } from "@/api/audioRoute";
import { useOneCallAtATime } from "@/api/calls";
import { ShareBlocksAcrossDeployments } from "@/api/identity";
import { WakeThisPhone } from "@/api/push";
import { NotifyOnMessages } from "@/features/notifications/NotifyOnMessages";
import { PlaySounds } from "@/features/notifications/PlaySounds";
import { SyncProvider } from "@/api/sync";
import { CommunityRail } from "@/features/communities/CommunityRail";
import { AttributionsScreen } from "@/features/about/AttributionsScreen";
import { DocumentLinks } from "@/features/documents/DocumentLinks";
import { DocumentScreen } from "@/features/documents/DocumentScreen";
import { DeploymentWelcome } from "@/features/auth/DeploymentWelcome";
import { DeviceLinkRoute } from "@/features/auth/DeviceLinkScreen";
import { LoginForm } from "@/features/auth/LoginForm";
import { ProviderReturn } from "@/features/auth/ProviderReturn";
import { RegisterForm } from "@/features/auth/RegisterForm";
import { ServerChoiceContext, useServerChoice } from "@/features/auth/serverChoice";
import { AccountSwitcher } from "@/features/accounts/AccountSwitcher";
import { NewRelease } from "@/features/layout/NewRelease";
import { UpdateReady } from "@/features/updates/UpdateReady";
import { SyncBanner } from "@/features/layout/SyncBanner";
import { EnrollmentScreen } from "@/features/security/EnrollmentScreen";
import { VerificationScreen } from "@/features/email/VerificationScreen";
import { FollowLanguagePreference } from "@/features/settings/LanguageSection";
import { SourcePickerDialog } from "@/features/voice/SourcePickerDialog";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";
import { IncomingCalls } from "@/features/voice/IncomingCall";

/**
 * Every account this install holds, each under its own scope so that those behind the one in front
 * stay connected, and what the device does once however many there are.
 */
export function RootLayout() {
  const accounts = useAccountsHub();
  const { entries, front } = useAccounts();
  const { href } = useLocation();
  useEffect(() => {
    accounts.noteRoute(href);
  }, [accounts, href]);
  return (
    <>
      <OnceOnThisDevice />
      {entries.map((entry) => (
        <AccountScope key={entry.account.id} entry={entry} several={entries.length > 1}>
          <AccountView front={entry.account.id === front} several={entries.length > 1} />
        </AccountScope>
      ))}
    </>
  );
}

/** One call at a time, the phone's wakings, and ringing, across every account. */
function OnceOnThisDevice() {
  useOneCallAtATime();
  useFollowSpeakerphone();
  return useAllSources().length === 0 ? null : (
    <>
      <WakeThisPhone />
      <IncomingCalls />
    </>
  );
}

function AccountScope({
  entry,
  several,
  children,
}: {
  entry: AccountEntry;
  several: boolean;
  children: ReactNode;
}) {
  const { account, client } = entry;
  const inherited = useContext(ServerChoiceContext);
  const choice = useMemo(
    () =>
      several || inherited === null
        ? { serverUrl: account.serverUrl, changeServer: null, switchServer: null }
        : inherited,
    [several, inherited, account.serverUrl],
  );
  return (
    <AccountContext.Provider value={account}>
      <ServerChoiceContext.Provider value={choice}>
        <HomeClientContext.Provider value={client}>
          <AspenClientContext.Provider value={client}>{children}</AspenClientContext.Provider>
        </HomeClientContext.Provider>
      </ServerChoiceContext.Provider>
    </AccountContext.Provider>
  );
}

/**
 * One account. In front and signed out: the sign-in or create-account screen, leaving the URL
 * alone so a shared link opens once the user is in. In front and signed in to an account that
 * still owes the server a second factor: the screen for adding one; or a verified email address:
 * the screen for verifying it. Signed in: its sync, and in front the app beside the community
 * rail. An account behind the front one that cannot run shows nothing until it is switched to.
 */
function AccountView({ front, several }: { front: boolean; several: boolean }) {
  const session = useSession();
  if (
    session !== null &&
    session.twoFactorEnrollmentRequired !== true &&
    session.emailVerificationRequired !== true
  ) {
    return <SignedIn front={front} />;
  }
  if (!front) {
    return null;
  }
  return (
    <>
      {several && (
        <div className="fixed end-3 top-3 z-10 flex">
          <AccountSwitcher />
        </div>
      )}
      {session === null ? (
        <SignedOut />
      ) : session.twoFactorEnrollmentRequired === true ? (
        <EnrollmentScreen />
      ) : (
        <VerificationScreen />
      )}
    </>
  );
}

/**
 * Signed out: the deployment's welcome above the sign-in or create-account form. An invite link
 * naming another deployment says the user's account may be on any deployment. A registration link (`/register`, with `?invite=` from the Administration
 * Dashboard) opens on the create-account screen with the invite filled in. A sign-in code
 * (`/device-link`) opens its own screen instead, as does the return from an identity provider
 * (`/sign-in/provider`) and Open Source Attributions (`/attributions`), which anyone may read, as
 * do the deployment's Privacy Policy (`/privacy`) and Terms of Use (`/terms`), which the sign-in
 * and create-account screens link to at their foot.
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
  // The browser back from an identity provider: the flow is claimed and finished there.
  if (pathname === "/sign-in/provider") {
    return <ProviderReturn />;
  }
  if (pathname === "/attributions") {
    return <AttributionsScreen signedOut />;
  }
  if (pathname === "/privacy" || pathname === "/terms") {
    return (
      <DocumentScreen kind={pathname === "/privacy" ? "privacyPolicy" : "termsOfUse"} signedOut />
    );
  }
  return (
    <main className="flex min-h-full flex-col items-center gap-4 p-6">
      <div className="flex w-full flex-1 flex-col items-center justify-center gap-4">
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
      </div>
      <footer>
        <DocumentLinks />
      </footer>
    </main>
  );
}

function FollowMotionSpeed() {
  useFollowMotionSpeed();
  return null;
}

function FollowMessageTextSize() {
  useFollowMessageTextSize();
  return null;
}

/** Tells the rest of the app this account's deployments, and how to sign it out. */
function PublishAccount() {
  const accounts = useAccountsHub();
  const { id } = useAccount();
  const sources = useSources();
  const signOut = useEndSignIns(sources);
  useEffect(() => {
    accounts.publish(id, { sources, signOut });
  }, [accounts, id, sources, signOut]);
  useEffect(
    () => () => {
      accounts.publish(id, null);
    },
    [accounts, id],
  );
  return null;
}

/** What runs for a signed-in account whether or not it is shown, and the app when it is. */
function SignedIn({ front }: { front: boolean }) {
  const client = useAspenClient();
  return (
    <SyncProvider client={client} attended={front}>
      <PublishAccount />
      <ShareBlocksAcrossDeployments />
      <NotifyOnMessages />
      <PlaySounds />
      {front && <ShownAccount />}
    </SyncProvider>
  );
}

/**
 * The signed-in app: the community rail beside the route. On a narrow screen a conversation
 * (a channel, a DM, a thread) takes the whole width, and the rail shows with the lists the
 * back links lead to.
 */
function ShownAccount() {
  const { channelId } = useParams({ strict: false });
  return (
    <>
      <FollowLanguagePreference />
      <FollowMotionSpeed />
      <FollowMessageTextSize />
      <Announcer />
      <div className="flex h-full flex-col">
        <SyncBanner />
        <NewRelease />
        <UpdateReady />
        <SourcePickerDialog />
        {/* Panels slide in from beyond the screen's edge; nothing there may widen the page. */}
        <div className="flex min-h-0 flex-1 overflow-x-clip">
          <div className={channelId === undefined ? "flex" : "hidden md:flex"}>
            <CommunityRail />
          </div>
          <Outlet />
        </div>
      </div>
    </>
  );
}
