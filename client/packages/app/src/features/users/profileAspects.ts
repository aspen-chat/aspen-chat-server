import type { ProfileAspect } from "@aspen/protocol";
import { useMessages } from "@/i18n/context";

/** Every aspect of a profile a report may name, in the order they are listed. */
export const PROFILE_ASPECTS: readonly ProfileAspect[] = [
  "displayName",
  "username",
  "picture",
  "status",
  "bio",
  "pronouns",
];

/** The aspects named, in their usual order and in the reader's language, as one line. */
export function useAspectList(): (aspects: readonly ProfileAspect[]) => string {
  const m = useMessages();
  return (aspects) =>
    PROFILE_ASPECTS.filter((a) => aspects.includes(a))
      .map((a) => m.reports.aspects[a])
      .join(", ");
}
