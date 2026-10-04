import type { Role } from "@aspen/protocol";
import { useRoleColor } from "@/features/users/nameColor";

/**
 * A dot in a role's colour beside its name, or nothing for a role without one; a community's
 * role or the deployment's.
 */
export function RoleSwatch({ role }: { role: Pick<Role, "hue"> }) {
  const color = useRoleColor(role);
  if (color === undefined) {
    return null;
  }
  return (
    <span
      aria-hidden="true"
      className="h-2 w-2 shrink-0 rounded-full"
      style={{ backgroundColor: color }}
    />
  );
}
