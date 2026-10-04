import {
  explain,
  type AccessDecision,
  type OverrideGrant,
  type Permission,
  type Role,
} from "@aspen/protocol";
import { CaretDownIcon, CheckIcon, XIcon } from "@phosphor-icons/react";
import { useState } from "react";
import {
  Button,
  Dialog,
  Label,
  ListBox,
  ListBoxItem,
  Modal,
  ModalOverlay,
  Popover,
  Select,
  SelectValue,
  ToggleButton,
  ToggleButtonGroup,
} from "react-aria-components";
import {
  useAccess,
  useCategoryOverrides,
  useChannel,
  useChannelOverrides,
  useRoles,
  useStore,
  useSync,
} from "@/api/hooks";
import { alertClass, fieldClass, hintClass, labelClass } from "@/features/auth/styles";
import { MemberPicker } from "@/features/community-settings/MemberPicker";
import {
  currentPreset,
  withheldBy,
  type Preset,
  type PresetOption,
} from "@/features/community-settings/accessPresets";
import { PresetChoices } from "@/features/community-settings/PresetChoices";
import { CHANNEL_GROUPS } from "@/features/community-settings/permissionGroups";
import {
  dialogClass,
  optionClass,
  overlayClass,
  secondaryButtonClass,
  selectButtonClass,
  wideModalClass,
} from "@/features/invites/dialog";
import { DialogHeading } from "@/features/layout/DialogHeading";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";
import { primaryButtonClass } from "@/features/auth/styles";
import { problemText } from "@/api/problemText";

/** A channel or a category, whose overrides the dialog edits. */
export type AccessTarget =
  | { kind: "channel"; id: string; name: string; communityId: string }
  | { kind: "category"; id: string; name: string; communityId: string };

/**
 * Who can use a channel, or every channel of a category. It leads with three plain choices
 * (everyone; only some roles; read-only except some roles), keeps per-role allow, default, and
 * deny for each permission under Advanced, and explains what any member can do there under
 * Check access.
 */
export function AccessDialog({
  target,
  isOpen,
  onOpenChange,
}: {
  target: AccessTarget;
  isOpen: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const m = useMessages();
  return (
    <ModalOverlay
      isOpen={isOpen}
      onOpenChange={onOpenChange}
      isDismissable
      className={overlayClass}
    >
      <Modal className={wideModalClass}>
        <Dialog className={dialogClass}>
          <DialogHeading>
            {target.kind === "channel"
              ? format(m.access.channelHeading, { channel: target.name })
              : format(m.access.categoryHeading, { category: target.name })}
          </DialogHeading>
          {target.kind === "category" && <p className={hintClass}>{m.access.categoryNote}</p>}
          <AccessEditor target={target} />
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}

function AccessEditor({ target }: { target: AccessTarget }) {
  const m = useMessages();
  const roles = useRoles(target.communityId);
  const channelOverrides = useChannelOverrides(target.kind === "channel" ? target.id : "");
  const categoryOverrides = useCategoryOverrides(target.kind === "category" ? target.id : "");
  const overrides = target.kind === "channel" ? channelOverrides : categoryOverrides;
  const everyone = roles.find((r) => r.everyone);
  return (
    <div className="flex flex-col gap-6">
      <Presets target={target} roles={roles} overrides={overrides} everyone={everyone} />
      <details className="flex flex-col gap-3">
        <summary className="cursor-pointer text-sm font-semibold text-ink-muted">
          {m.access.advanced}
        </summary>
        <Advanced target={target} roles={roles} overrides={overrides} />
      </details>
      <details className="flex flex-col gap-3">
        <summary className="cursor-pointer text-sm font-semibold text-ink-muted">
          {m.access.check}
        </summary>
        <CheckAccess target={target} />
      </details>
    </div>
  );
}

function Presets({
  target,
  roles,
  overrides,
  everyone,
}: {
  target: AccessTarget;
  roles: readonly Role[];
  overrides: readonly OverrideGrant[];
  everyone: Role | undefined;
}) {
  const m = useMessages();
  const sync = useSync();
  const current = currentPreset(overrides, everyone?.id);
  const [preset, setPreset] = useState<Preset>(current.preset);
  const [chosen, setChosen] = useState<ReadonlySet<string>>(() => new Set(current.roles));
  const [applying, setApplying] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const others = roles.filter((r) => !r.everyone).reverse();

  async function apply() {
    if (everyone === undefined || preset === "custom") {
      return;
    }
    setApplying(true);
    setError(null);
    // Everyone's override says who is shut out, each chosen role's lets it back in, and every
    // other override goes, so the result is exactly the preset. The roles are let in first, so
    // their members never lose the channel while the change is made.
    const withheld = withheldBy(preset);
    const set = (role: string, value: { allow: Permission[]; deny: Permission[] } | null) =>
      sync.setOverride(target.kind, target.id, role, value);
    const roleWrites = others.flatMap((role) =>
      preset !== "everyone" && chosen.has(role.id)
        ? [set(role.id, { allow: [...withheld], deny: [] })]
        : overrides.some((o) => o.role === role.id)
          ? [set(role.id, null)]
          : [],
    );
    const results = await Promise.allSettled(roleWrites);
    const failed = results.find((r) => r.status === "rejected");
    if (failed === undefined) {
      try {
        await set(everyone.id, withheld.length === 0 ? null : { allow: [], deny: [...withheld] });
      } catch (e) {
        setError(problemText(e));
      }
    } else {
      setError(problemText(failed.reason));
    }
    setApplying(false);
  }

  const options: readonly PresetOption[] = [
    { key: "everyone", label: m.access.presets.everyone, hint: m.access.presets.everyoneHint },
    { key: "private", label: m.access.presets.private, hint: m.access.presets.privateHint },
    { key: "readOnly", label: m.access.presets.readOnly, hint: m.access.presets.readOnlyHint },
    ...(current.preset === "custom"
      ? [
          {
            key: "custom" as const,
            label: m.access.presets.custom,
            hint: m.access.presets.customHint,
          },
        ]
      : []),
  ];

  return (
    <div className="flex flex-col gap-3">
      <PresetChoices
        options={options}
        preset={preset}
        onPresetChange={setPreset}
        roles={others}
        chosen={chosen}
        onChosenChange={setChosen}
      />
      {error !== null && (
        <p role="alert" className={alertClass}>
          {error}
        </p>
      )}
      <Button
        isDisabled={applying || preset === "custom"}
        onPress={() => {
          void apply();
        }}
        className={primaryButtonClass + " self-start"}
      >
        {applying ? m.access.applying : m.access.apply}
      </Button>
    </div>
  );
}

type Setting = "allow" | "inherit" | "deny";

/** One role's override, permission by permission: allowed, left to the role, or denied. */
function Advanced({
  target,
  roles,
  overrides,
}: {
  target: AccessTarget;
  roles: readonly Role[];
  overrides: readonly OverrideGrant[];
}) {
  const m = useMessages();
  const sync = useSync();
  const highestFirst = [...roles].reverse();
  const [roleId, setRoleId] = useState<string | null>(highestFirst.at(-1)?.id ?? null);
  const [draft, setDraft] = useState<ReadonlyMap<Permission, Setting> | null>(null);
  const [error, setError] = useState<string | null>(null);
  const saved = overrides.find((o) => o.role === roleId);
  const settingOf = (p: Permission): Setting =>
    draft?.get(p) ??
    (saved?.allow.includes(p) === true
      ? "allow"
      : saved?.deny.includes(p) === true
        ? "deny"
        : "inherit");
  const everything = CHANNEL_GROUPS.flatMap((g) => g.permissions);

  async function save(clear: boolean) {
    if (roleId === null) {
      return;
    }
    const allow = everything.filter((p) => settingOf(p) === "allow");
    const deny = everything.filter((p) => settingOf(p) === "deny");
    setError(null);
    try {
      await sync.setOverride(
        target.kind,
        target.id,
        roleId,
        clear || (allow.length === 0 && deny.length === 0) ? null : { allow, deny },
      );
      setDraft(null);
    } catch (e) {
      setError(problemText(e));
    }
  }

  return (
    <div className="flex flex-col gap-3 pt-2">
      <p className={hintClass}>{m.access.advancedHint}</p>
      <Select
        value={roleId}
        onChange={(key) => {
          setRoleId(key === null ? null : String(key));
          setDraft(null);
        }}
        className={fieldClass}
      >
        <Label className={labelClass}>{m.access.roleLabel}</Label>
        <Button className={selectButtonClass}>
          <SelectValue />
          <CaretDownIcon size={14} aria-hidden="true" />
        </Button>
        <Popover className="max-h-72 min-w-(--trigger-width) overflow-y-auto rounded-md border border-line bg-surface-raised p-1 shadow-lg">
          <ListBox items={highestFirst} className="outline-none">
            {(role) => (
              <ListBoxItem id={role.id} textValue={role.name} className={optionClass}>
                {role.everyone ? m.roles.everyone : role.name}
              </ListBoxItem>
            )}
          </ListBox>
        </Popover>
      </Select>
      {CHANNEL_GROUPS.map((group) => (
        <fieldset key={group.key} className="flex flex-col gap-1">
          <legend className="mb-1 text-sm font-semibold text-ink-muted">
            {m.permissionGroups[group.key]}
          </legend>
          {group.permissions.map((permission) => (
            <div key={permission} className="flex flex-wrap items-center gap-2">
              <span className="min-w-0 flex-1 text-sm">{m.permissionNames[permission].name}</span>
              <ToggleButtonGroup
                aria-label={m.permissionNames[permission].name}
                selectionMode="single"
                disallowEmptySelection
                selectedKeys={[settingOf(permission)]}
                onSelectionChange={(keys) => {
                  const [key] = Array.from(keys);
                  if (key === undefined) {
                    return;
                  }
                  const next = new Map(everything.map((p) => [p, settingOf(p)] as const));
                  next.set(permission, key as Setting);
                  setDraft(next);
                }}
                className="flex overflow-hidden rounded-md border border-line"
              >
                {(["allow", "inherit", "deny"] as const).map((setting) => (
                  <ToggleButton
                    key={setting}
                    id={setting}
                    className={
                      "px-2 py-1 text-xs outline-none hover:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50 " +
                      (setting === "allow"
                        ? "selected:bg-online/20 selected:text-ink"
                        : setting === "deny"
                          ? "selected:bg-danger-soft selected:text-danger"
                          : "selected:bg-surface-hover selected:text-ink")
                    }
                  >
                    {m.access[setting]}
                  </ToggleButton>
                ))}
              </ToggleButtonGroup>
            </div>
          ))}
        </fieldset>
      ))}
      {error !== null && (
        <p role="alert" className={alertClass}>
          {error}
        </p>
      )}
      <div className="flex flex-wrap gap-2">
        <Button
          isDisabled={draft === null}
          onPress={() => {
            void save(false);
          }}
          className={primaryButtonClass}
        >
          {m.access.saveOverride}
        </Button>
        {saved !== undefined && (
          <Button
            onPress={() => {
              void save(true);
            }}
            className={secondaryButtonClass}
          >
            {m.access.clearOverride}
          </Button>
        )}
      </div>
    </div>
  );
}

/** What a chosen member can do here, permission by permission, and what decided each. */
function CheckAccess({ target }: { target: AccessTarget }) {
  const m = useMessages();
  const store = useStore();
  const roles = useRoles(target.communityId);
  const [userId, setUserId] = useState<string | null>(null);
  const channel = useChannel(target.kind === "channel" ? target.id : "");
  const category = target.kind === "category" ? target.id : (channel?.parentCategory ?? undefined);
  const categoryOverrides = useCategoryOverrides(category ?? "");
  const channelOverrides = useChannelOverrides(target.kind === "channel" ? target.id : "");
  // Re-read when the caller's own access changes, since it may change the answers too.
  useAccess(target.communityId);
  const access = userId === null ? null : store.access(target.communityId, userId);
  const roleName = (id: string) => {
    const role = roles.find((r) => r.id === id);
    return role === undefined ? "?" : role.everyone ? m.roles.everyone : role.name;
  };
  const reasonText = (decision: AccessDecision): string => {
    const reason = decision.reason;
    switch (reason.kind) {
      case "owner":
        return m.access.reasons.owner;
      case "moderator":
        return m.access.reasons.moderator;
      case "roles":
        return format(m.access.reasons.roles, { roles: reason.roles.map(roleName).join(", ") });
      case "none":
        return m.access.reasons.noRole;
      case "override":
        return format(reason.allowed ? m.access.reasons.allowed : m.access.reasons.denied, {
          role: roleName(reason.role),
          layer:
            reason.layer === "channel"
              ? m.access.reasons.layerChannel
              : m.access.reasons.layerCategory,
        });
    }
  };
  return (
    <div className="flex flex-col gap-3 pt-2">
      <p className={hintClass}>{m.access.checkHint}</p>
      <MemberPicker
        communityId={target.communityId}
        label={m.access.memberLabel}
        onChange={(user) => {
          setUserId(user?.id ?? null);
        }}
      />
      {access !== null && (
        <table className="w-full text-sm">
          <tbody>
            {CHANNEL_GROUPS.flatMap((g) => g.permissions).map((permission) => {
              const decision = explain(
                access,
                permission,
                categoryOverrides,
                target.kind === "channel" ? channelOverrides : [],
              );
              return (
                <tr key={permission} className="border-t border-line align-top">
                  <th scope="row" className="py-1.5 pe-2 text-start font-normal">
                    {m.permissionNames[permission].name}
                  </th>
                  <td className="py-1.5 pe-2 whitespace-nowrap">
                    <span className="flex items-center gap-1">
                      {decision.allowed ? (
                        <CheckIcon size={14} aria-hidden="true" className="text-online" />
                      ) : (
                        <XIcon size={14} aria-hidden="true" className="text-danger" />
                      )}
                      {decision.allowed ? m.access.yes : m.access.no}
                    </span>
                  </td>
                  <td className="py-1.5 text-ink-muted">{reasonText(decision)}</td>
                </tr>
              );
            })}
          </tbody>
        </table>
      )}
    </div>
  );
}
