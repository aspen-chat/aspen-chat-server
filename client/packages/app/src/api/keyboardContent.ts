import { Capacitor, registerPlugin, type PluginListenerHandle } from "@capacitor/core";
import { useEffect, useRef, type RefObject } from "react";

/**
 * The Android app's native side of what a keyboard inserts besides text, a GIF, a sticker, or an
 * image (`AspenKeyboardContentPlugin`): the WebView tells the keyboard a field takes them only
 * while `setAccepting` says the focused field does, and each one inserted is copied into the
 * app's cache and announced as `content`, its `url` a `file:` URL of the copy, until `release`
 * removes it.
 */
interface AspenKeyboardContentPlugin {
  setAccepting(options: { accepting: boolean }): Promise<void>;
  release(options: { url: string }): Promise<void>;
  addListener(
    event: "content",
    listener: (content: { url: string; mimeType: string }) => void,
  ): Promise<PluginListenerHandle>;
}

const AspenKeyboardContent = registerPlugin<AspenKeyboardContentPlugin>("AspenKeyboardContent");

function available(): boolean {
  return Capacitor.isNativePlatform() && Capacitor.isPluginAvailable("AspenKeyboardContent");
}

/**
 * Offers what a keyboard inserts to `take` while `box` is focused and `on`, as an unnamed file.
 * Content inserted while no box that takes it is focused is let go.
 */
export function useKeyboardContent(
  box: RefObject<HTMLTextAreaElement | null>,
  on: boolean,
  take: (file: File) => void,
): void {
  const latest = useRef(take);
  useEffect(() => {
    latest.current = take;
  });
  useEffect(() => {
    const element = box.current;
    if (!on || element === null || !available()) {
      return;
    }
    const accept = (accepting: boolean) => {
      void AspenKeyboardContent.setAccepting({ accepting }).catch(() => undefined);
    };
    const onFocus = () => {
      accept(true);
    };
    const onBlur = () => {
      accept(false);
    };
    element.addEventListener("focus", onFocus);
    element.addEventListener("blur", onBlur);
    if (document.activeElement === element) {
      accept(true);
    }
    let live = true;
    let handle: PluginListenerHandle | null = null;
    void AspenKeyboardContent.addListener("content", ({ url, mimeType }) => {
      if (!live || document.activeElement !== element) {
        return;
      }
      void fetch(Capacitor.convertFileSrc(url))
        .then((response) => response.blob())
        .then((blob) => {
          if (live) {
            latest.current(new File([blob], "", { type: mimeType }));
          }
        })
        .catch(() => undefined)
        .finally(() => {
          void AspenKeyboardContent.release({ url }).catch(() => undefined);
        });
    }).then((added) => {
      if (live) {
        handle = added;
      } else {
        void added.remove();
      }
    });
    return () => {
      live = false;
      void handle?.remove();
      element.removeEventListener("focus", onFocus);
      element.removeEventListener("blur", onBlur);
      if (document.activeElement === element) {
        accept(false);
      }
    };
  }, [box, on]);
}
