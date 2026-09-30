import type { Permission, PermissionSet } from "@aspen/protocol";
import { ChoiceCheckbox, UNIFORM_CHOICE_CLASS } from "@/features/layout/choices";
import { useUniformHeight } from "@/features/layout/useUniformHeight";
import { PERMISSION_GROUPS } from "@/features/community-settings/permissionGroups";
import { useMessages } from "@/i18n/context";

/**
 * A role's permissions, grouped, one checkbox each with what it allows. A permission the
 * editor does not hold cannot be given or taken, so its box is disabled; `disabled` disables
 * them all. Every box is as tall as the tallest, in every group, so the list reads as an even
 * grid whatever the length of each hint (`useUniformHeight`).
 */
export function PermissionChecklist({
  value,
  onChange,
  held,
  disabled = false,
}: {
  value: ReadonlySet<Permission>;
  onChange: (next: ReadonlySet<Permission>) => void;
  /** What the editor holds. */
  held: PermissionSet;
  disabled?: boolean;
}) {
  const m = useMessages();
  const list = useUniformHeight();
  return (
    <div ref={list} className="flex flex-col gap-4">
      {PERMISSION_GROUPS.map((group) => (
        <fieldset key={group.key} className="flex flex-col gap-2">
          <legend className="mb-1 text-sm font-semibold text-ink-muted">
            {m.permissionGroups[group.key]}
          </legend>
          <div className="grid gap-2 sm:grid-cols-2">
            {group.permissions.map((permission) => (
              <ChoiceCheckbox
                key={permission}
                isSelected={value.has(permission)}
                isDisabled={disabled || !held.has(permission)}
                onChange={(selected) => {
                  const next = new Set(value);
                  if (selected) {
                    next.add(permission);
                  } else {
                    next.delete(permission);
                  }
                  onChange(next);
                }}
                label={m.permissionNames[permission].name}
                hint={m.permissionNames[permission].hint}
                className={UNIFORM_CHOICE_CLASS}
              />
            ))}
          </div>
        </fieldset>
      ))}
    </div>
  );
}
