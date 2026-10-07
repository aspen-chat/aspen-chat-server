import { Button } from "react-aria-components";

/** One choice on the first step of a stepped dialog: a title with a line of explanation. */
export function OptionButton({
  title,
  hint,
  onPress,
}: {
  title: string;
  hint: string;
  onPress: () => void;
}) {
  return (
    <Button
      onPress={onPress}
      className="flex flex-col gap-1 rounded-lg border border-line bg-surface px-4 py-3 text-start outline-none hover:border-accent hover:bg-surface-hover pressed:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50"
    >
      <span className="font-medium">{title}</span>
      <span className="text-sm text-ink-muted">{hint}</span>
    </Button>
  );
}
