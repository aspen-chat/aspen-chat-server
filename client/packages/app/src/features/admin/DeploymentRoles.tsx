import type { DeploymentPermission, DeploymentRole } from "@aspen/protocol";
import { ArrowDownIcon, ArrowUpIcon, PlusIcon } from "@phosphor-icons/react";
import { useState } from "react";
import { Button, GridList, GridListItem, Input, Label, TextField } from "react-aria-components";
import { useSync } from "@/api/hooks";
import { ReadFailed, Section } from "@/features/admin/AdminDashboard";
import type { AdminRead } from "@/features/admin/useAdminRead";
import {
  DEPLOYMENT_PERMISSIONS,
  rankOf,
  type DeploymentRoles,
} from "@/features/admin/deploymentRoleRecords";
import {
  alertClass,
  fieldClass,
  hintClass,
  inputClass,
  labelClass,
  primaryButtonClass,
} from "@/features/auth/styles";
import { dangerButtonClass, secondaryButtonClass } from "@/features/invites/dialog";
import { ChoiceCheckbox, UNIFORM_CHOICE_CLASS } from "@/features/layout/choices";
import { useUniformHeight } from "@/features/layout/useUniformHeight";
import { Tooltip } from "@/features/layout/Tooltip";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";
import { LoadingLabel, Skeleton } from "@/features/layout/Skeleton";
import { CopyIdButton } from "@/features/layout/CopyId";
import { problemText } from "@/api/problemText";

const iconButtonClass =
  "rounded p-1 text-ink-muted outline-none hover:bg-surface-hover hover:text-ink " +
  "disabled:opacity-30 focus-visible:ring-2 focus-visible:ring-accent/50";

/**
 * The deployment's roles, highest first, and the editor for the one chosen. Those who may manage
 * deployment roles create, move, change, and delete the roles below their own highest, giving
 * only permissions they hold; everyone else sees them as they are.
 */
export function DeploymentRolesSection({ read }: { read: AdminRead<DeploymentRoles> }) {
  const m = useMessages();
  const sync = useSync();
  const [selected, setSelected] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const data = read.data;
  const roles = data?.roles ?? [];
  const highestFirst = [...roles].reverse();
  const current = selected === null ? highestFirst[0] : roles.find((r) => r.id === selected);
  const rank = data === undefined ? 0 : rankOf(data);
  const manage = data?.mine.permissions.includes("manageDeploymentRoles") ?? false;
  const movable = roles.filter((r) => r.position < rank);

  function run(action: Promise<unknown>) {
    setError(null);
    action.then(read.reload, (e: unknown) => {
      setError(problemText(e));
    });
  }

  function move(role: DeploymentRole, by: -1 | 1) {
    const ids = movable.map((r) => r.id);
    const at = ids.indexOf(role.id);
    const other = ids[at + by];
    if (at < 0 || other === undefined) {
      return;
    }
    ids[at + by] = role.id;
    ids[at] = other;
    run(sync.admin.reorderDeploymentRoles(ids));
  }

  return (
    <Section id="admin-roles" title={m.admin.deploymentRoles} hint={m.admin.deploymentRolesHint}>
      {read.error !== null && <ReadFailed error={read.error} onRetry={read.reload} />}
      <div className="flex flex-col gap-4 md:flex-row">
        <div className="flex flex-col gap-2 md:w-56 md:shrink-0">
          {data === undefined && read.error === null && (
            <div aria-busy="true" className="flex flex-col gap-1">
              <LoadingLabel />
              {["w-28", "w-20", "w-24"].map((width) => (
                <Skeleton key={width} className={"my-1.5 h-4 " + width} />
              ))}
            </div>
          )}
          <GridList
            aria-label={m.admin.deploymentRoles}
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
            className="flex flex-col gap-0.5 outline-none"
          >
            {(role) => {
              const index = movable.findIndex((r) => r.id === role.id);
              return (
                <GridListItem
                  id={role.id}
                  textValue={role.name}
                  className="flex items-center gap-1 rounded-md px-2 py-1 text-sm outline-none hover:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50 selected:bg-accent-soft selected:text-accent-strong"
                >
                  <span className="min-w-0 flex-1 truncate">{role.name}</span>
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
                setError(null);
                sync.admin.createDeploymentRole(m.roles.newName, []).then(
                  (role) => {
                    setSelected(role.id);
                    read.reload();
                  },
                  (e: unknown) => {
                    setError(problemText(e));
                  },
                );
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
        {current !== undefined && data !== undefined && (
          <DeploymentRoleEditor
            key={current.id + current.name + current.permissions.join()}
            role={current}
            editable={manage && current.position < rank}
            held={new Set(data.mine.permissions)}
            onChanged={read.reload}
            onDeleted={() => {
              setSelected(null);
              read.reload();
            }}
          />
        )}
      </div>
    </Section>
  );
}

function DeploymentRoleEditor({
  role,
  editable,
  held,
  onChanged,
  onDeleted,
}: {
  role: DeploymentRole;
  editable: boolean;
  held: ReadonlySet<DeploymentPermission>;
  onChanged: () => void;
  onDeleted: () => void;
}) {
  const m = useMessages();
  const sync = useSync();
  const [name, setName] = useState(role.name);
  const [permissions, setPermissions] = useState<ReadonlySet<DeploymentPermission>>(
    () => new Set(role.permissions),
  );
  const permissionList = useUniformHeight();
  const [confirmingDelete, setConfirmingDelete] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const nameChanged = name.trim() !== role.name;
  const permissionsChanged =
    permissions.size !== role.permissions.length ||
    role.permissions.some((p) => !permissions.has(p));

  function save() {
    setSaving(true);
    setError(null);
    sync.admin
      .updateDeploymentRole(role.id, {
        ...(nameChanged ? { name: name.trim() } : {}),
        ...(permissionsChanged ? { permissions: Array.from(permissions) } : {}),
      })
      .then(onChanged, (e: unknown) => {
        setError(problemText(e));
      })
      .finally(() => {
        setSaving(false);
      });
  }

  return (
    <form
      onSubmit={(event) => {
        event.preventDefault();
        save();
      }}
      className="flex min-w-0 flex-1 flex-col gap-4"
    >
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
      {!editable && <p className={hintClass}>{m.roles.aboveYou}</p>}
      <div ref={permissionList} className="grid gap-2 sm:grid-cols-2">
        {DEPLOYMENT_PERMISSIONS.map((permission) => (
          <ChoiceCheckbox
            key={permission}
            isSelected={permissions.has(permission)}
            isDisabled={!editable || !held.has(permission)}
            onChange={(selected) => {
              const next = new Set(permissions);
              if (selected) {
                next.add(permission);
              } else {
                next.delete(permission);
              }
              setPermissions(next);
            }}
            label={m.deploymentPermissionNames[permission].name}
            hint={m.deploymentPermissionNames[permission].hint}
            className={UNIFORM_CHOICE_CLASS}
          />
        ))}
      </div>
      {error !== null && (
        <p role="alert" className={alertClass}>
          {error}
        </p>
      )}
      {editable && (
        <div className="flex flex-wrap items-center gap-2">
          <Button
            type="submit"
            isDisabled={saving || !(nameChanged || permissionsChanged)}
            className={primaryButtonClass}
          >
            {saving ? m.roles.saving : m.roles.save}
          </Button>
          <span className="ms-auto flex items-center gap-2">
            {confirmingDelete && (
              <span className="text-sm">{format(m.roles.deleteConfirm, { role: role.name })}</span>
            )}
            <Button
              onPress={() => {
                if (!confirmingDelete) {
                  setConfirmingDelete(true);
                  return;
                }
                sync.admin.deleteDeploymentRole(role.id).then(onDeleted, (e: unknown) => {
                  setError(problemText(e));
                });
              }}
              className={
                confirmingDelete ? dangerButtonClass : secondaryButtonClass + " text-danger"
              }
            >
              {m.roles.delete}
            </Button>
          </span>
        </div>
      )}
      <CopyIdButton id={role.id} thing="deploymentRole" className="self-end" />
    </form>
  );
}
