import { useCallback, useEffect, useRef, useSyncExternalStore } from "react";
import { useAllSources } from "./accounts";
import type { Source } from "./everywhere";

/** Whether a source's call is under way, or ended in a way the user still has to be told. */
function holdsCall(source: Source): boolean {
  const state = source.sync.voice.state;
  return state.status !== "idle" || state.endedReason !== null;
}

/**
 * The deployment whose call the user is in, or was just dropped from, under any account; `null`
 * when none. A person is in one call at a time, wherever it is (`useOneCallAtATime`).
 */
export function useCallSource(): Source | null {
  const sources = useAllSources();
  const version = useRef(0);
  const subscribe = useCallback(
    (listener: () => void) => {
      const stops = sources.map((source) =>
        source.sync.voice.subscribe(() => {
          version.current += 1;
          listener();
        }),
      );
      return () => {
        for (const stop of stops) {
          stop();
        }
      };
    },
    [sources],
  );
  useSyncExternalStore(subscribe, () => version.current);
  return sources.find(holdsCall) ?? null;
}

/**
 * Keeps the person in one call across every deployment of every account: joining a call on one
 * leaves any call on another.
 */
export function useOneCallAtATime(): void {
  const sources = useAllSources();
  const active = useRef(new Set<Source>());
  useEffect(() => {
    const notice = (source: Source) => {
      const joined = source.sync.voice.state.status !== "idle";
      if (joined && !active.current.has(source)) {
        for (const other of active.current) {
          if (other !== source) {
            other.sync.voice.leave();
          }
        }
        active.current = new Set([source]);
      } else if (!joined) {
        active.current.delete(source);
      }
    };
    const stops = sources.map((source) => {
      notice(source);
      return source.sync.voice.subscribe(() => {
        notice(source);
      });
    });
    return () => {
      for (const stop of stops) {
        stop();
      }
    };
  }, [sources]);
}
