import { LANGUAGE } from "@aspen/protocol";
import { CaretDownIcon } from "@phosphor-icons/react";
import { useCallback, useEffect, useSyncExternalStore } from "react";
import {
  Button,
  Label,
  ListBox,
  ListBoxItem,
  Popover,
  Select,
  SelectValue,
} from "react-aria-components";
import { useSync } from "@/api/hooks";
import { optionClass, selectButtonClass } from "@/features/invites/dialog";
import { useLanguageSetting, useMessages } from "@/i18n/context";
import { AUTOMATIC, LANGUAGES, type Language } from "@/i18n/locales";
import { format, type Messages } from "@/i18n/messages";

/** A language's name in itself, as its speakers would look for it. */
function nativeName(language: Language, m: Messages): string {
  switch (language) {
    case "en-XA":
      return m.settings.pseudoAccented;
    case "ar-XB":
      return m.settings.pseudoMirrored;
    default:
      return new Intl.DisplayNames([language], { type: "language" }).of(language) ?? language;
  }
}

/** The language the app shows, kept with the account. */
export function LanguageSection() {
  const m = useMessages();
  const sync = useSync();
  const { choice, setChoice, resolved } = useLanguageSetting();
  const options = [
    {
      id: AUTOMATIC,
      label: format(m.settings.languageAutomatic, {
        language: nativeName(resolved.language, m),
      }),
    },
    ...LANGUAGES.map((language) => ({ id: language, label: nativeName(language, m) })),
  ];
  return (
    <section aria-labelledby="settings-language" className="flex flex-col gap-3">
      <h3 id="settings-language" className="text-sm font-semibold text-ink-muted">
        {m.settings.language}
      </h3>
      <Select
        value={choice}
        onChange={(key) => {
          if (typeof key !== "string") {
            return;
          }
          setChoice(key);
          void sync.preferences.set(LANGUAGE, key).catch(() => undefined);
        }}
        className="flex flex-col gap-1"
      >
        <Label className="text-sm font-medium">{m.settings.languageLabel}</Label>
        <Button className={selectButtonClass}>
          <SelectValue className="truncate" />
          <CaretDownIcon size={14} aria-hidden="true" className="shrink-0 text-ink-faint" />
        </Button>
        <Popover className="min-w-(--trigger-width) rounded-md border border-line bg-surface-raised p-1 shadow-lg">
          <ListBox items={options}>
            {(option) => (
              <ListBoxItem id={option.id} textValue={option.label} className={optionClass}>
                {option.label}
              </ListBoxItem>
            )}
          </ListBox>
        </Popover>
      </Select>
      <p className="text-xs text-ink-muted">{m.settings.languageHint}</p>
    </section>
  );
}

/**
 * Shows the language the account chose, on every device it signs in on, once the account's
 * preferences have been read; until then the one this install last showed stays.
 */
export function FollowLanguagePreference() {
  const sync = useSync();
  const { setChoice } = useLanguageSetting();
  const subscribe = useCallback(
    (listener: () => void) => sync.preferences.subscribe(listener),
    [sync],
  );
  const chosen = useSyncExternalStore(subscribe, () =>
    sync.preferences.accountLoaded ? sync.preferences.get(LANGUAGE) : null,
  );
  useEffect(() => {
    if (chosen !== null) {
      setChoice(chosen);
    }
  }, [chosen, setChoice]);
  return null;
}
