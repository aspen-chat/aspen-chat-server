/** What the sync layer reaches of the page it runs in, when there is one. */

import type { PreferenceStorage } from "./preferences";
import type { VoiceMedia } from "./voiceMedia";

/**
 * The browser media, created only when a call first needs it, so the sync layer can be built
 * where there is no browser at all.
 */
export function lazyBrowserMedia(): VoiceMedia {
  let real: VoiceMedia | null = null;
  const media = async (): Promise<VoiceMedia> => {
    if (real === null) {
      const { browserVoiceMedia } = await import("./browserMedia");
      real = browserVoiceMedia();
    }
    return real;
  };
  return {
    createDevice: async () => (await media()).createDevice(),
    getMicrophone: async (choice) => (await media()).getMicrophone(choice),
    getCamera: async (choice) => (await media()).getCamera(choice),
    setOutput: async (choice) => (await media()).setOutput(choice),
    getScreen: async () => (await media()).getScreen(),
    setVolume: (consumerId, gain) => {
      if (real !== null) {
        real.setVolume(consumerId, gain);
      }
    },
    play: (id, track) => {
      real?.play(id, track);
    },
    stop: (id) => {
      real?.stop(id);
    },
  };
}

/** The page's `localStorage`, when there is one and it can be touched. */
export function pageStorage(): PreferenceStorage | null {
  try {
    return typeof window === "undefined" ? null : window.localStorage;
  } catch {
    return null;
  }
}
