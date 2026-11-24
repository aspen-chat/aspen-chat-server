import { Button, Label, RadioButton, RadioField, RadioGroup, Text } from "react-aria-components";
import { planeClass, secondaryButtonClass } from "@/features/invites/dialog";
import { choiceClass, RadioMark } from "@/features/layout/choices";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";
import { useDesktopUpdates } from "./useDesktopUpdates";

const CHOICES: readonly DesktopUpdateChoice[] = ["stable", "beta", "off"];

/**
 * Settings' updates, in the desktop build that updates itself and nowhere else: which releases
 * it follows, or none, and how far it has got. An administrator's policy or the installer's flag
 * turns updating off beyond the reader's reach, which the section says instead of offering a
 * choice it would not honour.
 */
export function UpdatesSection() {
  const m = useMessages();
  const state = useDesktopUpdates();
  const updates = window.aspenDesktop?.updates;
  if (state === null || updates === undefined) {
    return null;
  }
  const status = (() => {
    switch (state.status) {
      case "idle":
        return null;
      case "checking":
        return m.updates.checking;
      case "downloading":
        return m.updates.downloading;
      case "ready":
        return format(m.updates.ready, { version: state.version ?? "" });
      case "failed":
        return m.updates.failed;
    }
  })();
  return (
    <section aria-labelledby="settings-updates" className={planeClass}>
      <h3 id="settings-updates" className="text-lg font-semibold text-ink-muted">
        {m.updates.title}
      </h3>
      {state.locked === null ? (
        <RadioGroup
          value={state.choice}
          onChange={(value) => {
            const choice = CHOICES.find((candidate) => candidate === value);
            if (choice !== undefined) {
              updates.choose(choice).catch((error: unknown) => {
                console.error("the update choice could not be kept", error);
              });
            }
          }}
          className="flex flex-col gap-2"
        >
          <Label className="text-sm font-semibold text-ink-muted">{m.updates.follow}</Label>
          <Text slot="description" className="text-xs text-ink-muted">
            {m.updates.hint}
          </Text>
          {CHOICES.map((choice) => (
            <RadioField key={choice} value={choice}>
              <RadioButton className={choiceClass}>
                <RadioMark />
                <span className="flex flex-col">
                  <span className="font-medium">{m.updates.choices[choice]}</span>
                  <span className="text-xs text-ink-muted">{m.updates.choiceHints[choice]}</span>
                </span>
              </RadioButton>
            </RadioField>
          ))}
        </RadioGroup>
      ) : (
        <p className="text-sm text-ink-muted">
          {state.locked === "administrator"
            ? m.updates.lockedAdministrator
            : m.updates.lockedInstaller}
        </p>
      )}
      {status !== null && (
        <div role="status" className="flex flex-wrap items-center gap-2">
          <p className="text-xs text-ink-muted">{status}</p>
          {state.status === "ready" && (
            <Button
              onPress={() => {
                updates.restart();
              }}
              className={secondaryButtonClass}
            >
              {m.updates.restart}
            </Button>
          )}
        </div>
      )}
    </section>
  );
}
