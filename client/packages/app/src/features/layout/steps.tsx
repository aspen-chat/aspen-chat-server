import { ArrowLeftIcon } from "@phosphor-icons/react";
import type { ReactNode } from "react";
import { Button } from "react-aria-components";

import { DialogHeading } from "@/features/layout/DialogHeading";
import { useMessages } from "@/i18n/context";

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
      className="flex flex-col gap-1 rounded-lg border border-line bg-surface px-4 py-3 text-left outline-none hover:border-accent hover:bg-surface-hover pressed:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50"
    >
      <span className="font-medium">{title}</span>
      <span className="text-sm text-ink-muted">{hint}</span>
    </Button>
  );
}

/** The heading of a later step, with a control that returns to the choice. */
export function StepHeading({ onBack, children }: { onBack: () => void; children: ReactNode }) {
  const m = useMessages();
  return (
    <div className="flex items-center gap-2">
      <Button
        onPress={onBack}
        aria-label={m.back}
        className="rounded-md p-1 text-ink-muted outline-none hover:text-ink focus-visible:ring-2 focus-visible:ring-accent/50"
      >
        <ArrowLeftIcon size={18} aria-hidden="true" />
      </Button>
      <DialogHeading>{children}</DialogHeading>
    </div>
  );
}
