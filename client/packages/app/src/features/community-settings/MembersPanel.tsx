import { ApiProblemError, type Role, type User } from "@aspen/protocol";
import { CaretDownIcon, CheckIcon, CrownSimpleIcon } from "@phosphor-icons/react";
import { useState } from "react";
import {
  Button,
  CheckboxButton,
  CheckboxField,
  Dialog,
  DialogTrigger,
  Input,
  Label,
  Popover,
  SearchField,
} from "react-aria-components";
import {
  useAccess,
  useCommunity,
  useMe,
  useMemberRoles,
  useRoles,
  useStore,
  useSync,
} from "@/api/hooks";
import { alertClass, fieldClass, hintClass, inputClass, labelClass } from "@/features/auth/styles";
import { useMemberSearch } from "@/features/community-settings/memberSearch";
import { Avatar } from "@/features/communities/Avatar";
import { dangerButtonClass, secondaryButtonClass } from "@/features/invites/dialog";
import { markClass } from "@/features/layout/choices";
import { displayNameOf } from "@/features/users/profile";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

function problemText(e: unknown): string {
  return e instanceof ApiProblemError ? e.message : String(e);
}

/**
 * The community's members, each with their roles: the member sample, or, for those who may search
 * every member, whoever the search finds. Those who
 * may assign roles change a member's roles below their own highest; those who may remove
 * members remove anyone ranked below them, never the owner.
 */
export function MembersPanel({ communityId }: { communityId: string }) {
  const m = useMessages();
  const [query, setQuery] = useState("");
  const { members, searching, error, canSearch } = useMemberSearch(communityId, query);
  return (
    <div className="flex flex-col gap-2">
      {canSearch && (
        <SearchField value={query} onChange={setQuery} className={fieldClass + " max-w-sm"}>
          <Label className={labelClass}>{m.members.searchLabel}</Label>
          <Input className={inputClass} />
        </SearchField>
      )}
      <p className={hintClass}>
        {query.trim() !== "" && canSearch ? m.members.searchHint : m.members.sampleNote}
      </p>
      {error !== null && (
        <p role="alert" className={alertClass}>
          {error}
        </p>
      )}
      {searching && <p className={hintClass}>{m.loading}</p>}
      {!searching && members.length === 0 && <p className={hintClass}>{m.members.noneFound}</p>}
      <ul aria-label={m.members.listLabel} className="flex flex-col gap-1">
        {members.map((member) => (
          <MemberRow key={member.id} communityId={communityId} member={member} />
        ))}
      </ul>
    </div>
  );
}

function MemberRow({ communityId, member }: { communityId: string; member: User }) {
  const m = useMessages();
  const sync = useSync();
  const store = useStore();
  const me = useMe();
  const community = useCommunity(communityId);
  const access = useAccess(communityId);
  const roles = useRoles(communityId);
  const held = useMemberRoles(communityId, member.id);
  const [confirming, setConfirming] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const name = displayNameOf(member);
  const owner = community?.owner === member.id;
  const self = me?.id === member.id;
  const theirs = store.access(communityId, member.id);
  const outranked = access !== null && theirs !== null && access.outranks(theirs.rank);
  const mayRemove = access?.has("removeMembers") === true && !self && !owner && outranked;
  const mayAssign = access?.has("assignRoles") === true && (self || (!owner && outranked));
  const shown = roles.filter((r) => !r.everyone && (held ?? []).includes(r.id));

  async function remove() {
    setError(null);
    try {
      await sync.removeMember(communityId, member.id);
    } catch (e) {
      setError(problemText(e));
    }
  }

  return (
    <li className="flex flex-col gap-1 rounded-md px-2 py-1.5 hover:bg-surface-hover/60">
      <div className="flex flex-wrap items-center gap-2">
        <Avatar name={name} iconId={member.icon} size="sm" />
        <span className="truncate font-medium">
          {name}
          {self && <span className="font-normal text-ink-muted"> ({m.members.you})</span>}
        </span>
        {owner && (
          <span className="flex items-center gap-1 text-xs text-ink-muted">
            <CrownSimpleIcon size={14} aria-hidden="true" />
            {m.members.owner}
          </span>
        )}
        <span className="flex flex-wrap gap-1">
          {shown.map((role) => (
            <span
              key={role.id}
              className="rounded-full border border-line px-2 py-0.5 text-xs text-ink-muted"
            >
              {role.name}
            </span>
          ))}
        </span>
        <span className="ml-auto flex items-center gap-2">
          {mayAssign && (
            <RolePicker
              communityId={communityId}
              userId={member.id}
              name={name}
              roles={roles.filter((r) => !r.everyone)}
              held={held ?? []}
              canGive={(role) => access.outranks(role.position)}
              onError={setError}
            />
          )}
          {mayRemove &&
            (confirming ? (
              <Button
                onPress={() => {
                  void remove();
                }}
                className={dangerButtonClass}
              >
                {m.members.remove}
              </Button>
            ) : (
              <Button
                onPress={() => {
                  setConfirming(true);
                }}
                className={secondaryButtonClass + " text-danger"}
              >
                {m.members.remove}
              </Button>
            ))}
        </span>
      </div>
      {confirming && (
        <p className="text-sm text-ink-muted">
          {format(m.members.removeConfirm, { name, community: community?.name ?? "" })}
        </p>
      )}
      {error !== null && (
        <p role="alert" className={alertClass}>
          {error}
        </p>
      )}
    </li>
  );
}

/** A member's roles as checkboxes, highest first; only roles below the caller's can change. */
function RolePicker({
  communityId,
  userId,
  name,
  roles,
  held,
  canGive,
  onError,
}: {
  communityId: string;
  userId: string;
  name: string;
  roles: readonly Role[];
  held: readonly string[];
  canGive: (role: Role) => boolean;
  onError: (message: string | null) => void;
}) {
  const m = useMessages();
  const sync = useSync();
  const label = format(m.members.rolesOf, { name });
  return (
    <DialogTrigger>
      <Button aria-label={label} className={secondaryButtonClass + " flex items-center gap-1"}>
        {m.members.editRoles}
        <CaretDownIcon size={12} aria-hidden="true" />
      </Button>
      <Popover
        placement="bottom end"
        className="w-64 rounded-md border border-line bg-surface-raised p-2 shadow-lg"
      >
        <Dialog aria-label={label} className="flex flex-col gap-1 outline-none">
          {[...roles].reverse().map((role) => (
            <CheckboxField
              key={role.id}
              isSelected={held.includes(role.id)}
              isDisabled={!canGive(role)}
              onChange={(selected) => {
                onError(null);
                sync.setMemberRole(communityId, userId, role.id, selected).catch((e: unknown) => {
                  onError(problemText(e));
                });
              }}
            >
              <CheckboxButton className="group flex items-center gap-2 rounded px-2 py-1 text-sm outline-none hover:bg-surface-hover disabled:opacity-50 focus-visible:ring-2 focus-visible:ring-accent/50">
                <span className={markClass + " mt-0"}>
                  <CheckIcon
                    size={12}
                    weight="bold"
                    aria-hidden="true"
                    className="hidden group-selected:block"
                  />
                </span>
                <span className="truncate">{role.name}</span>
              </CheckboxButton>
            </CheckboxField>
          ))}
        </Dialog>
      </Popover>
    </DialogTrigger>
  );
}
