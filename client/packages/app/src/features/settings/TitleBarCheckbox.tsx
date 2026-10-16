import { useEffect, useState } from "react";
import { Button } from "react-aria-components";
import { secondaryButtonClass } from "@/features/invites/dialog";
import { ChoiceCheckbox } from "@/features/layout/choices";
import { useMessages } from "@/i18n/context";

/**
 * Whether the desktop app draws its own title bar (`TitleBar.tsx`), offered only where the
 * system's may be kept instead (Linux; `packages/desktop/src/main/titleBar.ts`). A window's frame
 * is set as it is made, so a change applies from the next launch, which the reader may start
 * at once.
 */
export function TitleBarCheckbox() {
  const m = useMessages();
  const [choice, setChoice] = useState<{ drawn: boolean; next: boolean } | null>(null);
  useEffect(() => {
    window.aspenDesktop?.titleBar
      .choice()
      .then(setChoice)
      .catch((error: unknown) => {
        console.error("the title bar's setting could not be read", error);
      });
  }, []);
  const desktop = window.aspenDesktop?.titleBar;
  if (desktop === undefined || choice === null) {
    return null;
  }
  return (
    <div className="flex flex-col gap-2">
      <ChoiceCheckbox
        isSelected={choice.next}
        onChange={(next) => {
          setChoice({ ...choice, next });
          desktop.choose(next).catch((error: unknown) => {
            console.error("the title bar's setting could not be kept", error);
          });
        }}
        label={m.settings.titleBar}
        hint={m.settings.titleBarHint}
      />
      {choice.next !== choice.drawn && (
        <div className="flex flex-wrap items-center gap-2">
          <p className="text-xs text-ink-muted">{m.settings.titleBarRestart}</p>
          <Button
            onPress={() => {
              desktop.relaunch();
            }}
            className={secondaryButtonClass}
          >
            {m.settings.restartNow}
          </Button>
        </div>
      )}
    </div>
  );
}
