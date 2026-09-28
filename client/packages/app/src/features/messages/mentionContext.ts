import type { Mentions } from "@aspen/protocol";
import { createContext } from "react";

/** The message a body belongs to, for its tags: which count, and where to find role names. */
export const MentionContext = createContext<{
  mentions: Mentions;
  communityId: string | null;
} | null>(null);
