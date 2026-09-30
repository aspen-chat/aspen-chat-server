import { useIcon } from "@/api/hooks";

/**
 * A round picture for a user or community: their icon when they have one and it has loaded,
 * otherwise a circle with the initials of their name. Icons are records fetched by id on
 * demand, so the initials show until the record arrives.
 */
export function Avatar({
  name,
  iconId,
  size = "md",
}: {
  name: string;
  iconId?: string | null | undefined;
  size?: "xs" | "sm" | "md" | "lg";
}) {
  const icon = useIcon(iconId ?? undefined);
  const dimensions = {
    lg: "h-12 w-12 text-base",
    md: "h-9 w-9 text-sm",
    sm: "h-6 w-6 text-xs",
    xs: "h-5 w-5 text-[0.5rem]",
  }[size];
  if (icon !== undefined) {
    return (
      <img
        src={icon.downloadUrl}
        alt=""
        aria-hidden="true"
        draggable={false}
        className={`${dimensions} shrink-0 rounded-full bg-surface-sunken object-cover select-none`}
      />
    );
  }
  return (
    <span
      aria-hidden="true"
      className={`flex ${dimensions} shrink-0 items-center justify-center rounded-full bg-accent-soft font-semibold text-accent-strong select-none`}
    >
      {initials(name)}
    </span>
  );
}

function initials(name: string): string {
  const words = name
    .trim()
    .split(/\s+/)
    .filter((w) => w.length > 0);
  const first = words[0]?.[0] ?? "?";
  const second = words.length > 1 ? (words[words.length - 1]?.[0] ?? "") : "";
  return (first + second).toUpperCase();
}
