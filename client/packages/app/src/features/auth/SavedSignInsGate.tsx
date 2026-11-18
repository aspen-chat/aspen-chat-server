import { useEffect, useState, type ReactNode } from "react";
import { Button } from "react-aria-components";
import {
  continueWithoutSaving,
  openSavedSignIns,
  savedSignInsOpen,
  startSavedSignInsOver,
  type SavedSignIns,
} from "@/api/sessions";
import { primaryButtonClass } from "@/features/auth/styles";
import { secondaryButtonClass } from "@/features/invites/dialog";
import { LoadingLabel } from "@/features/layout/Skeleton";
import { useMessages } from "@/i18n/context";

/**
 * Renders `children` once the sign-ins saved on this device are read, which in the desktop and
 * mobile apps waits on the system's secure storage, and on a locked keyring's password prompt.
 * When they cannot be read it says why, and nothing is ever saved unencrypted in their place.
 */
export function SavedSignInsGate({ children }: { children: ReactNode }) {
  const m = useMessages().savedSignIns;
  const [state, setState] = useState<SavedSignIns | "opening">(() =>
    savedSignInsOpen() ? "open" : "opening",
  );
  useEffect(() => {
    if (state !== "opening") {
      return;
    }
    let current = true;
    void openSavedSignIns().then((opened) => {
      if (current) {
        setState(opened);
      }
    });
    return () => {
      current = false;
    };
  }, [state]);
  if (state === "open") {
    return children;
  }
  if (state === "opening") {
    return (
      <main aria-busy="true" className="min-h-full">
        <LoadingLabel text={m.opening} />
      </main>
    );
  }
  const unreadable = state === "unreadable";
  return (
    <main className="flex min-h-full items-center justify-center p-6">
      <section
        aria-labelledby="saved-sign-ins"
        className="flex max-w-md flex-col items-center gap-3 text-center"
      >
        <h1 id="saved-sign-ins" className="text-lg font-semibold">
          {unreadable ? m.unreadableHeading : m.unavailableHeading}
        </h1>
        <p className="text-sm text-ink-muted">
          {unreadable ? m.unreadableBody : m.unavailableBody}
        </p>
        <div className="flex flex-wrap justify-center gap-2">
          <Button
            className={primaryButtonClass}
            onPress={() => {
              setState("opening");
            }}
          >
            {m.tryAgain}
          </Button>
          <Button
            className={secondaryButtonClass}
            onPress={() => {
              if (unreadable) {
                void startSavedSignInsOver().then(() => {
                  setState("open");
                });
              } else {
                continueWithoutSaving();
                setState("open");
              }
            }}
          >
            {unreadable ? m.startOver : m.continueWithoutSaving}
          </Button>
        </div>
      </section>
    </main>
  );
}
