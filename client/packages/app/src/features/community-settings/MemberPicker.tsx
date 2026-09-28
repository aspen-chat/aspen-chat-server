import type { User } from "@aspen/protocol";
import { CaretDownIcon } from "@phosphor-icons/react";
import { useState } from "react";
import {
  Button,
  ComboBox,
  Input,
  Label,
  ListBox,
  ListBoxItem,
  Popover,
} from "react-aria-components";
import { fieldClass, inputClass, labelClass } from "@/features/auth/styles";
import { useMemberSearch } from "@/features/community-settings/memberSearch";
import { optionClass } from "@/features/invites/dialog";
import { displayNameOf } from "@/features/users/profile";
import { useMessages } from "@/i18n/context";

/**
 * Chooses one member of a community by typing their name: the member sample until something is
 * typed, then a search of every member for those who may search (`useMemberSearch`).
 */
export function MemberPicker({
  communityId,
  label,
  exclude = [],
  onChange,
}: {
  communityId: string;
  label: string;
  /** Members not to offer, such as the caller. */
  exclude?: readonly string[];
  onChange: (user: User | null) => void;
}) {
  const m = useMessages();
  const [query, setQuery] = useState("");
  const [chosen, setChosen] = useState<User | null>(null);
  // While the field shows the chosen member's name, the list is not narrowed by it.
  const typing = chosen === null || query !== displayNameOf(chosen);
  const search = useMemberSearch(communityId, typing ? query : "");
  const members = search.members.filter((u) => !exclude.includes(u.id));
  return (
    <ComboBox
      items={members}
      inputValue={query}
      onInputChange={setQuery}
      value={chosen?.id ?? null}
      onChange={(key) => {
        const user = members.find((u) => u.id === key) ?? null;
        setChosen(user);
        if (user !== null) {
          setQuery(displayNameOf(user));
        }
        onChange(user);
      }}
      menuTrigger="focus"
      allowsEmptyCollection
      className={fieldClass}
    >
      <Label className={labelClass}>{label}</Label>
      <div className="relative">
        <Input className={inputClass + " w-full pr-9"} />
        <Button className="absolute top-1/2 right-2 -translate-y-1/2 rounded p-1 text-ink-muted outline-none">
          <CaretDownIcon size={14} aria-hidden="true" />
        </Button>
      </div>
      <Popover className="max-h-72 min-w-(--trigger-width) overflow-y-auto rounded-md border border-line bg-surface-raised p-1 shadow-lg">
        <ListBox
          className="outline-none"
          renderEmptyState={() => (
            <p className="px-2 py-1 text-sm text-ink-muted">
              {search.error ?? (search.searching ? m.loading : m.members.noneFound)}
            </p>
          )}
        >
          {(user: User) => (
            <ListBoxItem id={user.id} textValue={displayNameOf(user)} className={optionClass}>
              {displayNameOf(user)}
              <span className="ml-2 text-xs text-ink-muted">{user.name}</span>
            </ListBoxItem>
          )}
        </ListBox>
      </Popover>
    </ComboBox>
  );
}
