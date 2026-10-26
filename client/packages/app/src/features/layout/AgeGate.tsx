import { useEffect, useState, type ReactNode } from "react";
import { canAskAge, deviceReportsUnderage } from "@/api/ageGate";
import { useMessages } from "@/i18n/context";

/**
 * Aspen is for adults only. Before anything else of the app shows, the mobile apps ask the
 * device whether its user is under 18 (`deviceReportsUnderage`); where it says so, this message
 * is all the app shows. Where the device reports no age, `children` show once it answers; on the
 * web and the desktop app, which cannot ask, they show at once.
 */
export function AgeGate({ children }: { children: ReactNode }) {
  const m = useMessages();
  const [underage, setUnderage] = useState<boolean | undefined>(() =>
    canAskAge() ? undefined : false,
  );
  useEffect(() => {
    if (!canAskAge()) {
      return;
    }
    let current = true;
    void deviceReportsUnderage().then((answer) => {
      if (current) {
        setUnderage(answer);
      }
    });
    return () => {
      current = false;
    };
  }, []);
  if (underage === undefined) {
    return <main aria-busy="true" className="min-h-full bg-surface" />;
  }
  if (underage) {
    return (
      <main className="flex min-h-full items-center justify-center bg-surface p-6">
        <p role="alert" className="max-w-sm text-center text-balance">
          {m.ageGate.underage}
        </p>
      </main>
    );
  }
  return children;
}
