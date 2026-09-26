import { Outlet } from "@tanstack/react-router";
import { useState } from "react";
import { useAspenClient, useSession } from "@/api/context";
import { SyncProvider } from "@/api/sync";
import { CommunityRail } from "@/features/communities/CommunityRail";
import { LoginForm } from "@/features/auth/LoginForm";
import { RegisterForm } from "@/features/auth/RegisterForm";
import { useServerChoice } from "@/features/auth/serverChoice";
import { SyncBanner } from "@/features/layout/SyncBanner";

/**
 * Signed out: the sign-in or create-account screen, leaving the URL alone so a shared link
 * opens once the user is in. Signed in: the community rail beside whatever the route shows,
 * all fed by one `SyncProvider`.
 */
export function RootLayout() {
  const session = useSession();
  if (session === null) {
    return <SignedOut />;
  }
  return <SignedIn />;
}

function SignedOut() {
  const { serverUrl, changeServer } = useServerChoice();
  const [screen, setScreen] = useState<"login" | "register">("login");
  return (
    <main className="flex min-h-full items-center justify-center p-6">
      {screen === "register" ? (
        <RegisterForm
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

function SignedIn() {
  const client = useAspenClient();
  return (
    <SyncProvider client={client}>
      <div className="flex h-full flex-col">
        <SyncBanner />
        <div className="flex min-h-0 flex-1">
          <CommunityRail />
          <Outlet />
        </div>
      </div>
    </SyncProvider>
  );
}
