import { ClockIcon, XIcon } from "@phosphor-icons/react";
import { Button } from "react-aria-components";
import { Tooltip } from "@/features/layout/Tooltip";
import { barButtonClass } from "@/features/messages/ReplyBar";
import { SCHEDULED_TIME_FORMAT } from "@/features/messages/scheduleTimes";
import { useMessages } from "@/i18n/context";
import { useDateFormat } from "@/i18n/format";
import { format } from "@/i18n/messages";

/**
 * Over the message box while what is written there is to be sent at a chosen time rather than
 * now: the time, and at its end a way to send now after all. The time is changed with the
 * box's schedule control, which opens its dialog on this time.
 */
export function ScheduleBar({ at, onCancel }: { at: Date; onCancel: () => void }) {
  const m = useMessages();
  const shown = useDateFormat(SCHEDULED_TIME_FORMAT);
  return (
    <div
      data-schedule-bar
      className="flex items-center gap-2 rounded-md bg-surface-sunken ps-2 text-sm text-ink-muted"
    >
      <ClockIcon size={16} aria-hidden="true" className="shrink-0" />
      <p className="min-w-0 flex-1 truncate">
        {format(m.scheduled.schedulingFor, { time: shown.format(at) })}
      </p>
      <Tooltip text={m.scheduled.cancel}>
        <Button aria-label={m.scheduled.cancel} onPress={onCancel} className={barButtonClass}>
          <XIcon size={14} aria-hidden="true" />
        </Button>
      </Tooltip>
    </div>
  );
}
