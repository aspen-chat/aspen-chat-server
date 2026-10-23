import {
  ApiProblemError,
  parseProviderFragment,
  type AccountOffer,
  type AspenClient,
  type IdentityProviderOutcome,
  type PendingAccount,
  type ProviderReturn as Returned,
  type SecondFactorMethod,
} from "@aspen/protocol";
import { useRouter } from "@tanstack/react-router";
import { useEffect, useState } from "react";
import { Button } from "react-aria-components";
import { useAspenClient, useSession } from "@/api/context";
import { LoadingLabel, Skeleton } from "@/features/layout/Skeleton";
import { toast } from "@/features/layout/toast";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";
import { takeFlow, type StartedFlow } from "./identityProviders";
import { SecondFactorStep } from "./LoginForm";
import { usePasskeyTransport } from "./passkeyTransport";
import { ProviderAccountStep } from "./ProviderAccountStep";
import { alertClass, primaryButtonClass } from "./styles";

/** A flow's claim, and the flow this tab kept for it. */
interface Claimed {
  outcome: IdentityProviderOutcome;
  started: StartedFlow;
}

/** The flow kept for a return is not in this tab: it started elsewhere, or long ago. */
class NotStartedHere extends Error {}

/**
 * Each flow claimed by this page, by id: a claim is answered once, so a screen that mounts
 * twice (React's development double render, or the route drawn again once signed in) reuses
 * the one claim.
 */
const claims = new Map<string, Promise<Claimed>>();

/**
 * Where the signed-in app opens once a sign-in this page finished has stored its session: the
 * page the tab was on when the flow started.
 */
let landing: string | null = null;

function claimOnce(client: AspenClient, returned: Returned): Promise<Claimed> {
  let claim = claims.get(returned.flow);
  if (claim === undefined) {
    const started = takeFlow(returned.flow);
    claim =
      started === null
        ? Promise.reject(new NotStartedHere())
        : client
            .claimIdentityProviderFlow(returned.flow, started.verifier, returned.code)
            .then((outcome) => ({ outcome, started }));
    claims.set(returned.flow, claim);
  }
  return claim;
}

type Phase =
  | { kind: "claiming"; providerName: string | null }
  | { kind: "failed"; message: string }
  | { kind: "secondFactor"; ticket: string; methods: SecondFactorMethod[] }
  | { kind: "account"; offer: AccountOffer; account: PendingAccount };

/**
 * The web client's `/sign-in/provider`, where the server sends the browser back from an
 * identity provider with the flow and its return code in the fragment, which is taken off the
 * address at once. It claims the flow with the verifier this tab kept, and then:
 *
 * - signed in, the app opens where the tab was when the flow started;
 * - a second factor owed is asked for, as after a password;
 * - an account to make asks for its username (`ProviderAccountStep`);
 * - a link or a fresh verification is said in a toast, back where the tab was;
 * - a failure shows the server's reason, with a way back.
 *
 * Opened without a fragment (once the session it made is stored, or by hand), it goes on to
 * where the sign-in started, or the home screen.
 */
export function ProviderReturn() {
  const m = useMessages();
  const client = useAspenClient();
  const router = useRouter();
  const signedIn = useSession() !== null;
  const transport = usePasskeyTransport();
  const [returned] = useState(() => parseProviderFragment(window.location.hash));
  const [phase, setPhase] = useState<Phase>({ kind: "claiming", providerName: null });

  useEffect(() => {
    if (returned === null) {
      const to = landing ?? "/";
      landing = null;
      router.history.replace(to);
      return;
    }
    // The return code is spent once claimed; it stays out of history and the address bar.
    if (window.location.hash !== "") {
      window.history.replaceState(
        window.history.state,
        "",
        window.location.pathname + window.location.search,
      );
    }
    let live = true;
    claimOnce(client, returned).then(
      ({ outcome, started }) => {
        if (!live) {
          return;
        }
        switch (outcome.outcome) {
          case "signedIn":
            // The session is stored; the signed-in app draws this route again, without the
            // fragment, and goes on.
            landing = started.returnPath;
            if (signedIn) {
              router.history.replace(started.returnPath);
            }
            break;
          case "secondFactorRequired":
            landing = started.returnPath;
            setPhase({ kind: "secondFactor", ticket: outcome.ticket, methods: outcome.methods });
            break;
          case "accountRequired":
            landing = started.returnPath;
            setPhase({
              kind: "account",
              offer: outcome.offer,
              account: outcome.account,
            });
            break;
          case "linked":
            toast(format(m.identityProviders.linked, { provider: outcome.identity.providerName }));
            router.history.replace(started.returnPath);
            break;
          case "reauthenticated":
            toast(format(m.identityProviders.verified, { provider: started.providerName }));
            router.history.replace(started.returnPath);
            break;
        }
      },
      (e: unknown) => {
        if (!live) {
          return;
        }
        setPhase({
          kind: "failed",
          message:
            e instanceof NotStartedHere
              ? m.identityProviders.notStartedHere
              : e instanceof ApiProblemError
                ? e.message
                : String(e),
        });
      },
    );
    return () => {
      live = false;
    };
  }, [client, returned, router, signedIn, m]);

  const leave = (to: string) => {
    landing = null;
    router.history.replace(to);
  };

  switch (phase.kind) {
    case "claiming":
      return (
        <Frame>
          <div aria-busy="true" className="flex w-full max-w-sm flex-col gap-3">
            <h1 className="text-2xl font-semibold">{m.identityProviders.finishingHeading}</h1>
            <LoadingLabel text={m.identityProviders.finishing} />
            <Skeleton className="h-10 w-full" />
          </div>
        </Frame>
      );
    case "failed":
      return (
        <Frame>
          <div className="flex w-full max-w-sm flex-col gap-4">
            <h1 className="text-2xl font-semibold">{m.identityProviders.failedHeading}</h1>
            <p role="alert" className={alertClass}>
              {phase.message}
            </p>
            <Button
              onPress={() => {
                leave("/");
              }}
              className={primaryButtonClass}
            >
              {signedIn ? m.continue : m.identityProviders.signInAnotherWay}
            </Button>
          </div>
        </Frame>
      );
    case "secondFactor":
      return (
        <Frame>
          <SecondFactorStep
            ticket={phase.ticket}
            methods={phase.methods}
            transport={transport}
            onStartOver={(reason) => {
              if (reason === null) {
                leave("/");
              } else {
                setPhase({ kind: "failed", message: reason });
              }
            }}
          />
        </Frame>
      );
    case "account":
      return (
        <Frame>
          <ProviderAccountStep
            offer={phase.offer}
            account={phase.account}
            onCancel={() => {
              leave("/");
            }}
          />
        </Frame>
      );
  }
}

/** The page around each phase: the whole screen, signed out, or beside the rail, signed in. */
function Frame({ children }: { children: React.ReactNode }) {
  return (
    <main className="flex min-h-full flex-1 flex-col items-center justify-center gap-4 p-6">
      {children}
    </main>
  );
}
