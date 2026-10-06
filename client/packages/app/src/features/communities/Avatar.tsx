import { useState } from "react";
import { useIcon, useIconLoading } from "@/api/hooks";
import { mediaUrl } from "@/features/layout/safeUrl";

/**
 * A round picture for a user or community: their icon when they have one, otherwise a circle
 * with the initials of their name. Icons are records fetched by id on demand; while the record
 * or its picture is on its way the circle pulses as a skeleton, so the initials never flash
 * before a picture replaces them. An icon that is gone, or whose picture will not load, shows
 * the initials.
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
  const src = mediaUrl(icon?.downloadUrl);
  const iconLoading = useIconLoading(iconId ?? undefined);
  const [shown, setShown] = useState<string | null>(null);
  const [failed, setFailed] = useState<string | null>(null);
  // Initials fill a circle of a set size, so they keep their size however large the text
  // around them is drawn (`type-scale` in `styles.css`).
  const dimensions = {
    lg: "h-12 w-12 text-[1rem]",
    md: "h-9 w-9 text-[0.875rem]",
    sm: "h-7 w-7 text-[0.75rem]",
    xs: "h-5 w-5 text-[0.5rem]",
  }[size];
  const skeleton = "animate-pulse bg-surface-hover motion-reduce:animate-none";
  if (icon === undefined && iconLoading) {
    return (
      <span
        aria-hidden="true"
        className={`block ${dimensions} shrink-0 rounded-full ${skeleton}`}
      />
    );
  }
  if (src !== undefined && failed !== src) {
    return (
      <img
        src={src}
        alt=""
        aria-hidden="true"
        draggable={false}
        onLoad={() => {
          setShown(src);
        }}
        onError={() => {
          setFailed(src);
        }}
        className={
          `${dimensions} shrink-0 rounded-full object-cover select-none ` +
          (shown === src ? "bg-surface-sunken" : skeleton)
        }
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
