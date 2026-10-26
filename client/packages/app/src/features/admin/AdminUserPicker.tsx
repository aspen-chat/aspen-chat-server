import { ApiProblemError, type AdminUserEntry } from "@aspen/protocol";
import { CaretDownIcon } from "@phosphor-icons/react";
import { useEffect, useState } from "react";
import {
  Button,
  ComboBox,
  Input,
  Label,
  ListBox,
  ListBoxItem,
  Popover,
} from "react-aria-components";
import { useSync } from "@/api/hooks";
import { fieldClass, inputClass, labelClass } from "@/features/auth/styles";
import { optionClass } from "@/features/invites/dialog";
import { RowsSkeleton } from "@/features/layout/ScreenSkeletons";
import { useMessages } from "@/i18n/context";

/** How long typing pauses before the search is sent. */
const SEARCH_DELAY_MS = 300;
/** How many people a search offers. */
const OFFERED = 10;

function nameOf(user: AdminUserEntry): string {
  return user.displayName ?? user.name;
}

/**
 * Chooses one of the deployment's users by typing their username or display name, searched in
 * the dashboard's user directory, for whoever may read it. Emptying the field chooses no one.
 */
export function AdminUserPicker({
  label,
  onChange,
}: {
  label: string;
  onChange: (user: AdminUserEntry | null) => void;
}) {
  const m = useMessages();
  const sync = useSync();
  const [query, setQuery] = useState("");
  const [chosen, setChosen] = useState<AdminUserEntry | null>(null);
  // What the last search found, and for what text; a search is under way while that is not
  // what the field asks for.
  const [result, setResult] = useState<{
    of: string | null;
    users: readonly AdminUserEntry[];
    error: string | null;
  }>({ of: null, users: [], error: null });
  // While the field shows the chosen person's name, the list is not narrowed by it.
  const typed = chosen !== null && query === nameOf(chosen) ? "" : query.trim();
  const searching = result.of !== typed;
  const found = result.users;
  const error = result.error;
  useEffect(() => {
    let current = true;
    const timer = setTimeout(() => {
      sync.admin.adminUsers({ ...(typed === "" ? {} : { name: typed }), limit: OFFERED }).then(
        (users) => {
          if (current) {
            setResult({ of: typed, users, error: null });
          }
        },
        (e: unknown) => {
          if (current) {
            setResult({
              of: typed,
              users: [],
              error: e instanceof ApiProblemError ? e.message : String(e),
            });
          }
        },
      );
    }, SEARCH_DELAY_MS);
    return () => {
      current = false;
      clearTimeout(timer);
    };
  }, [sync, typed]);
  return (
    <ComboBox
      items={found}
      inputValue={query}
      onInputChange={(text) => {
        setQuery(text);
        if (text.trim() === "" && chosen !== null) {
          setChosen(null);
          onChange(null);
        }
      }}
      value={chosen?.id ?? null}
      onChange={(key) => {
        const user = found.find((u) => u.id === key) ?? null;
        setChosen(user);
        if (user !== null) {
          setQuery(nameOf(user));
        }
        onChange(user);
      }}
      menuTrigger="focus"
      allowsEmptyCollection
      className={fieldClass}
    >
      <Label className={labelClass}>{label}</Label>
      <div className="relative">
        <Input className={inputClass + " w-full pe-9"} />
        <Button className="absolute top-1/2 end-2 -translate-y-1/2 rounded p-1 text-ink-muted outline-none">
          <CaretDownIcon size={14} aria-hidden="true" />
        </Button>
      </div>
      <Popover className="max-h-72 min-w-(--trigger-width) overflow-y-auto rounded-md border border-line bg-surface-raised p-1 shadow-lg">
        <ListBox
          className="outline-none"
          renderEmptyState={() =>
            searching && error === null ? (
              <RowsSkeleton count={3} />
            ) : (
              <p className="px-2 py-1 text-sm text-ink-muted">{error ?? m.members.noneFound}</p>
            )
          }
        >
          {(user: AdminUserEntry) => (
            <ListBoxItem id={user.id} textValue={nameOf(user)} className={optionClass}>
              {nameOf(user)}
              <span className="ms-2 text-xs text-ink-muted">
                {user.homeDomain == null ? user.name : `${user.name}@${user.homeDomain}`}
              </span>
            </ListBoxItem>
          )}
        </ListBox>
      </Popover>
    </ComboBox>
  );
}
