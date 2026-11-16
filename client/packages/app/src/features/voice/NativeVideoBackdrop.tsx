import { useSyncExternalStore } from "react";
import { nativeCalls } from "@/api/nativeCall";
import { nativeVideoLayout, type HoleRect } from "@/features/voice/nativeVideo";

/**
 * Paints the page's ground beneath everything with every native video hole cut out, while there
 * are holes, so a hole's ancestors, which `styles.css` leaves unpainted under `data-video-holes`,
 * still read as the page they were (`nativeVideo.ts`). Mounted once, at the app's root, in the
 * mobile apps.
 */
export function NativeVideoBackdrop() {
  const rects = useSyncExternalStore(
    (listener) => nativeVideoLayout.subscribe(listener),
    () => nativeVideoLayout.rects,
    () => nativeVideoLayout.rects,
  );
  if (!nativeCalls() || rects.length === 0) {
    return null;
  }
  return (
    <div
      aria-hidden="true"
      data-video-backdrop
      className="pointer-events-none fixed inset-0 -z-10 bg-surface"
      style={{ clipPath: holesClipPath(rects) }}
    />
  );
}

/**
 * The whole viewport less `rects`, as an even-odd polygon: the viewport's outline, then each
 * hole's, so the hole's inside is crossed twice and left out.
 */
function holesClipPath(rects: readonly HoleRect[]): string {
  const px = (value: number) => `${String(value)}px`;
  const points = ["0 0", "100% 0", "100% 100%", "0 100%", "0 0"];
  for (const rect of rects) {
    const left = px(rect.x);
    const top = px(rect.y);
    const right = px(rect.x + rect.width);
    const bottom = px(rect.y + rect.height);
    points.push(
      `${left} ${top}`,
      `${left} ${bottom}`,
      `${right} ${bottom}`,
      `${right} ${top}`,
      `${left} ${top}`,
    );
  }
  return `polygon(evenodd, ${points.join(", ")})`;
}
