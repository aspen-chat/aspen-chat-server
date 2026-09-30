import type { PreferenceDefinition } from "@aspen/protocol";

/**
 * A pane's width as this device keeps it. `version` is the pane's layout version: a width kept
 * under another version is set aside for the default, so an update that changes a pane enough
 * to need it can reset that pane's width, and only that pane's, by raising its version.
 */
export interface PaneWidth {
  readonly version: number;
  readonly width: number;
}

/** A pane whose width the reader sets by dragging its edge, and this device remembers. */
export interface PaneSizing {
  readonly definition: PreferenceDefinition<PaneWidth | null>;
  readonly version: number;
  readonly fallback: number;
  readonly min: number;
  readonly max: number;
}

export function paneSizing(
  key: string,
  version: number,
  { fallback, min, max }: { fallback: number; min: number; max: number },
): PaneSizing {
  return {
    definition: {
      key: `layout.${key}.width`,
      scope: "device",
      fallback: null,
      parse: (raw) => {
        const kept = raw as Partial<PaneWidth> | null;
        return kept !== null &&
          typeof kept === "object" &&
          kept.version === version &&
          typeof kept.width === "number" &&
          Number.isFinite(kept.width)
          ? { version, width: kept.width }
          : undefined;
      },
    },
    version,
    fallback,
    min,
    max,
  };
}

/** The channel list, and the DM list in its place. */
export const CHANNEL_LIST = paneSizing("channelList", 1, { fallback: 256, min: 200, max: 440 });
export const MEMBER_LIST = paneSizing("memberList", 1, { fallback: 224, min: 180, max: 400 });
export const THREAD_PANEL = paneSizing("threadPanel", 1, { fallback: 384, min: 300, max: 760 });
