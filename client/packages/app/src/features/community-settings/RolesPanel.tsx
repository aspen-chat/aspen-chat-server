import { TEMPLATES, type CommunityPermissions, type Permission, type Role } from "@aspen/protocol";
import { ArrowDownIcon, ArrowUpIcon, PlusIcon } from "@phosphor-icons/react";
import { useState } from "react";
import { Button, GridList, GridListItem, Input, Label, TextField } from "react-aria-components";
import { useAccess, useRoles, useSync, useUser } from "@/api/hooks";
import {
  alertClass,
  fieldClass,
  hintClass,
  inputClass,
  labelClass,
  primaryButtonClass,
} from "@/features/auth/styles";
import { PermissionChecklist } from "@/features/community-settings/PermissionChecklist";
import {
  dangerButtonClass,
  planeClass,
  planeSurfaceClass,
  secondaryButtonClass,
} from "@/features/invites/dialog";
import { ChoiceCheckbox } from "@/features/layout/choices";
import { Tooltip } from "@/features/layout/Tooltip";
import { HuePicker } from "@/features/users/HuePicker";
import { RoleSwatch } from "@/features/users/RoleSwatch";
import { displayNameOf } from "@/features/users/profile";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";
import { CopyIdButton } from "@/features/layout/CopyId";
import { problemText } from "@/api/problemText";

const iconButtonClass =
  "rounded p-1 text-ink-muted outline-none hover:bg-surface-hover hover:text-ink " +
  "disabled:opacity-30 focus-visible:ring-2 focus-visible:ring-accent/50";

/**
 * A community's roles, highest first, and the editor for the one chosen. Roles ranked below the
 * caller's highest can be moved, renamed, given or stripped of permissions the caller holds,
 * and deleted; everyone's role, always last, keeps its name and place.
 */
export function RolesPanel({ communityId }: { communityId: string }) {
  const m = useMessages();
  const sync = useSync();
  const roles = useRoles(communityId);
  const access = useAccess(communityId);
  const highestFirst = [...roles].reverse();
  const [selected, setSelected] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const manage = access?.has("manageRoles") ?? false;
  const current = selected === null ? highestFirst[0] : roles.find((r) => r.id === selected);
  // Roles the caller may move, lowest first, as the server takes their order.
  const movable = roles.filter((r) => !r.everyone && (access?.outranks(r.position) ?? false));

  function move(role: Role, by: -1 | 1) {
    const ids = movable.map((r) => r.id);
    const at = ids.indexOf(role.id);
    const to = at + by;
    const other = ids[to];
    if (at < 0 || other === undefined) {
      return;
    }
    ids[to] = role.id;
    ids[at] = other;
    setError(null);
    sync.reorderRoles(communityId, ids).catch((e: unknown) => {
      setError(problemText(e));
    });
  }

  async function create() {
    setError(null);
    try {
      const role = await sync.createRole(communityId, m.roles.newName, []);
      setSelected(role.id);
    } catch (e) {
      setError(problemText(e));
    }
  }

  return (
    <div className="flex flex-col gap-4 md:flex-row">
      <div className="flex flex-col gap-2 md:sticky md:top-0 md:w-56 md:shrink-0 md:self-start">
        <GridList
          aria-label={m.roles.listLabel}
          items={highestFirst}
          selectionMode="single"
          disallowEmptySelection
          selectedKeys={current === undefined ? [] : [current.id]}
          onSelectionChange={(keys) => {
            if (keys !== "all") {
              const [key] = Array.from(keys);
              setSelected(key === undefined ? null : String(key));
            }
          }}
          className={planeSurfaceClass + " flex flex-col gap-0.5 outline-none"}
        >
          {(role) => {
            const index = movable.findIndex((r) => r.id === role.id);
            return (
              <GridListItem
                id={role.id}
                textValue={role.name}
                className="flex items-center gap-1 rounded-md px-2 py-1 text-sm outline-none hover:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50 selected:bg-accent-soft selected:text-accent-strong"
              >
                <RoleSwatch role={role} />
                <span className="min-w-0 flex-1 truncate">
                  {role.everyone ? m.roles.everyone : role.name}
                </span>
                {manage && index >= 0 && (
                  <>
                    <Tooltip text={format(m.roles.moveUp, { role: role.name })}>
                      <Button
                        aria-label={format(m.roles.moveUp, { role: role.name })}
                        isDisabled={index === movable.length - 1}
                        onPress={() => {
                          move(role, 1);
                        }}
                        className={iconButtonClass}
                      >
                        <ArrowUpIcon size={14} aria-hidden="true" />
                      </Button>
                    </Tooltip>
                    <Tooltip text={format(m.roles.moveDown, { role: role.name })}>
                      <Button
                        aria-label={format(m.roles.moveDown, { role: role.name })}
                        isDisabled={index === 0}
                        onPress={() => {
                          move(role, -1);
                        }}
                        className={iconButtonClass}
                      >
                        <ArrowDownIcon size={14} aria-hidden="true" />
                      </Button>
                    </Tooltip>
                  </>
                )}
              </GridListItem>
            );
          }}
        </GridList>
        {manage && (
          <Button
            onPress={() => {
              void create();
            }}
            className={secondaryButtonClass + " flex items-center justify-center gap-1"}
          >
            <PlusIcon size={14} aria-hidden="true" />
            {m.roles.create}
          </Button>
        )}
        {error !== null && (
          <p role="alert" className={alertClass}>
            {error}
          </p>
        )}
      </div>
      {current !== undefined && access !== null && (
        <RoleEditor
          key={current.id}
          role={current}
          access={access}
          onDeleted={() => {
            setSelected(null);
          }}
        />
      )}
    </div>
  );
}

/**
 * One role's name, permissions, colour, and whether it is shown apart, edited as a draft and
 * saved together. The draft follows the role when someone else changes it and nothing is
 * changed here yet.
 */
function RoleEditor({
  role,
  access,
  onDeleted,
}: {
  role: Role;
  access: CommunityPermissions;
  onDeleted: () => void;
}) {
  const m = useMessages();
  const sync = useSync();
  const editable = access.has("manageRoles") && access.outranks(role.position);
  // The draft is `null` until something is changed here, and the role as saved shows until
  // then, whoever changes it.
  const [draft, setDraft] = useState<RoleDraft | null>(null);
  const current: RoleDraft = draft ?? {
    name: role.name,
    permissions: new Set(role.permissions),
    hue: role.hue ?? null,
    hoist: role.hoist ?? false,
  };
  const { name, permissions, hue, hoist } = current;
  const change = (next: Partial<RoleDraft>) => {
    setDraft({ ...current, ...next });
  };
  const setName = (next: string) => {
    change({ name: next });
  };
  const setPermissions = (next: ReadonlySet<Permission>) => {
    change({ permissions: next });
  };
  const [saving, setSaving] = useState(false);
  const [confirmingDelete, setConfirmingDelete] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const nameChanged = !role.everyone && name.trim() !== role.name;
  const permissionsChanged =
    permissions.size !== role.permissions.length ||
    role.permissions.some((p) => !permissions.has(p));
  const hueChanged = hue !== (role.hue ?? null);
  const hoistChanged = hoist !== (role.hoist ?? false);
  const dirty = nameChanged || permissionsChanged || hueChanged || hoistChanged;

  async function save() {
    setSaving(true);
    setError(null);
    try {
      await sync.updateRole(role.id, {
        ...(nameChanged ? { name: name.trim() } : {}),
        ...(permissionsChanged ? { permissions: Array.from(permissions) } : {}),
        ...(hueChanged ? { hue } : {}),
        ...(hoistChanged ? { hoist } : {}),
      });
      setDraft(null);
    } catch (e) {
      setError(problemText(e));
    } finally {
      setSaving(false);
    }
  }

  async function remove() {
    setError(null);
    try {
      await sync.deleteRole(role.id);
      onDeleted();
    } catch (e) {
      setError(problemText(e));
    }
  }

  // A template sets the permissions the editor holds, and leaves the rest as they were.
  function applyTemplate(template: readonly Permission[]) {
    const next = new Set(permissions);
    for (const permission of access.held) {
      if (template.includes(permission)) {
        next.add(permission);
      } else {
        next.delete(permission);
      }
    }
    setPermissions(next);
  }

  return (
    <form
      onSubmit={(event) => {
        event.preventDefault();
        void save();
      }}
      className={planeClass + " min-w-0 flex-1"}
    >
      {role.bot != null && <BotRoleNote botId={role.bot} />}
      {role.everyone ? (
        <p className={hintClass}>{m.roles.everyoneHint}</p>
      ) : (
        <TextField
          value={name}
          onChange={setName}
          isDisabled={!editable}
          maxLength={64}
          className={fieldClass}
        >
          <Label className={labelClass}>{m.roles.nameLabel}</Label>
          <Input className={inputClass} />
        </TextField>
      )}
      {!editable && <p className={hintClass}>{m.roles.aboveYou}</p>}
      {!role.everyone && (
        <>
          <HuePicker
            hue={hue}
            onChange={(next) => {
              change({ hue: next });
            }}
            sample={name.trim() === "" ? role.name : name.trim()}
            hint={m.roles.colorHint}
            isDisabled={!editable}
          />
          <ChoiceCheckbox
            isSelected={hoist}
            onChange={(next) => {
              change({ hoist: next });
            }}
            isDisabled={!editable}
            label={m.roles.hoistLabel}
            hint={m.roles.hoistHint}
          />
        </>
      )}
      {editable && (
        <div className="flex flex-wrap items-center gap-2">
          <span className="text-sm text-ink-muted">{m.roles.templatesLabel}</span>
          {(["member", "moderator", "admin"] as const).map((template) => (
            <Button
              key={template}
              onPress={() => {
                applyTemplate(TEMPLATES[template]);
              }}
              className={secondaryButtonClass}
            >
              {m.roles.templates[template]}
            </Button>
          ))}
        </div>
      )}
      {editable && !access.owner && <p className={hintClass}>{m.roles.notHeld}</p>}
      <PermissionChecklist
        value={permissions}
        onChange={setPermissions}
        held={access.held}
        disabled={!editable}
      />
      {error !== null && (
        <p role="alert" className={alertClass}>
          {error}
        </p>
      )}
      {editable && (
        <div className="flex flex-wrap items-center gap-2">
          <Button type="submit" isDisabled={!dirty || saving} className={primaryButtonClass}>
            {saving ? m.roles.saving : m.roles.save}
          </Button>
          {dirty && <span className="text-sm text-ink-muted">{m.roles.unsaved}</span>}
          {!role.everyone && role.bot == null && (
            <span className="ms-auto flex items-center gap-2">
              {confirmingDelete ? (
                <>
                  <span className="text-sm">
                    {format(m.roles.deleteConfirm, { role: role.name })}
                  </span>
                  <Button
                    onPress={() => {
                      void remove();
                    }}
                    className={dangerButtonClass}
                  >
                    {m.roles.delete}
                  </Button>
                </>
              ) : (
                <Button
                  onPress={() => {
                    setConfirmingDelete(true);
                  }}
                  className={secondaryButtonClass + " text-danger"}
                >
                  {m.roles.delete}
                </Button>
              )}
            </span>
          )}
          <CopyIdButton id={role.id} thing="role" className="ms-auto" />
        </div>
      )}
    </form>
  );
}

/** A role as the editor holds it while it is being changed. */
interface RoleDraft {
  name: string;
  permissions: ReadonlySet<Permission>;
  hue: number | null;
  hoist: boolean;
}

/** Says whose a bot's role is: it cannot be given to anyone else, and goes when the bot does. */
function BotRoleNote({ botId }: { botId: string }) {
  const m = useMessages();
  const bot = useUser(botId);
  return (
    <p className={hintClass}>
      {format(m.bots.roleOfBot, {
        name: bot === undefined ? m.unknownUser : displayNameOf(bot),
      })}
    </p>
  );
}
