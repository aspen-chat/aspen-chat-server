import { useNavigate } from "@tanstack/react-router";
import { useMemo } from "react";
import { useSync } from "@/api/hooks";
import { channelLink, useDomain } from "@/features/messages/links";

/** Ways to reach someone directly, each settling once it has or failing with why. */
export interface Reach {
  /** Opens the caller's DM with them, making it the first time. */
  readonly message: (userId: string) => Promise<void>;
  /** Opens that DM and joins its call, starting it when no one is in it yet. */
  readonly call: (userId: string) => Promise<void>;
}

/** Messaging and calling someone, as their profile card and their menu offer it. */
export function useReach(): Reach {
  const sync = useSync();
  const navigate = useNavigate();
  const domain = useDomain();
  return useMemo(() => {
    const open = async (userId: string) => {
      const dm = await sync.openDm([userId]);
      void navigate(channelLink({ domain, community: null }, dm.id));
      return dm.id;
    };
    return {
      message: async (userId) => {
        await open(userId);
      },
      call: async (userId) => {
        await sync.voice.join(await open(userId));
      },
    };
  }, [sync, navigate, domain]);
}
