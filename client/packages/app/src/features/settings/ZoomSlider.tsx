import { StepSlider } from "@/features/layout/StepSlider";
import { useNumberFormat } from "@/i18n/format";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";
import { Capacitor } from "@capacitor/core";
import { detectShell } from "@/config";
import { ZOOM_STEPS, setZoom, useZoom, zoomAvailable } from "@/theme/zoom";

/** The step nearest `factor`, for a factor the shell kept from elsewhere. */
function nearestStep(factor: number): number {
  return ZOOM_STEPS.reduce((nearest, step) =>
    Math.abs(step - factor) < Math.abs(nearest - factor) ? step : nearest,
  );
}

/**
 * How large the whole app is drawn in the desktop app (`theme/zoom.ts`), stepping as Ctrl + and
 * Ctrl − do. Where the app has no zoom of its own it says what does: on a phone, its own text and
 * display size settings, which the app follows; in a browser, the browser's zoom.
 */
export function ZoomSlider() {
  const m = useMessages();
  const factor = useZoom();
  const percent = useNumberFormat({ style: "percent", maximumFractionDigits: 0 });
  if (!zoomAvailable) {
    const hint =
      detectShell() !== "mobile"
        ? m.settings.zoomInBrowser
        : Capacitor.getPlatform() === "ios"
          ? m.settings.zoomOnIos
          : m.settings.zoomOnAndroid;
    return <p className="text-xs text-ink-muted">{hint}</p>;
  }
  if (factor === null) {
    return null;
  }
  const key = window.aspenDesktop?.platform === "darwin" ? "⌘" : "Ctrl";
  return (
    <StepSlider
      label={m.settings.zoom}
      values={ZOOM_STEPS}
      value={nearestStep(factor)}
      describe={(step) => percent.format(step)}
      onChoose={setZoom}
      hint={format(m.settings.zoomKeysHint, { key })}
    />
  );
}
