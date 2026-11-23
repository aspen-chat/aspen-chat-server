import { Button } from "react-aria-components";

/** The strip above the app that says a newer Aspen is ready and offers the one way into it. */
export function ReleaseStrip({
  text,
  action,
  onAction,
}: {
  text: string;
  action: string;
  onAction: () => void;
}) {
  return (
    <div
      role="status"
      className="motion-drop flex items-center justify-center gap-3 bg-accent-soft px-3 py-1 text-sm text-ink"
    >
      <span>{text}</span>
      <Button
        onPress={onAction}
        className="rounded border border-current px-2 py-0.5 text-xs outline-none pressed:opacity-70 focus-visible:ring-2 focus-visible:ring-accent/50"
      >
        {action}
      </Button>
    </div>
  );
}
