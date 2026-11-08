import type { CalendarDateTime } from "@internationalized/date";
import { useState } from "react";
import {
  Button,
  DateField,
  DateInput,
  DateSegment,
  Dialog,
  Form,
  Label,
  Modal,
  ModalOverlay,
  RadioButton,
  RadioField,
  RadioGroup,
} from "react-aria-components";
import {
  alertClass,
  fieldClass,
  inputClass,
  labelClass,
  primaryButtonClass,
} from "@/features/auth/styles";
import { dialogClass, modalClass, overlayClass } from "@/features/invites/dialog";
import { RadioMark, choiceClass } from "@/features/layout/choices";
import { DialogHeading } from "@/features/layout/DialogHeading";
import {
  describeTimeProblem,
  localDateTime,
  MAX_AHEAD_MS,
  MIN_AHEAD_MS,
  quickTimes,
  SCHEDULED_TIME_FORMAT,
  timeProblem,
  toDate,
} from "@/features/messages/scheduleTimes";
import { useMessages } from "@/i18n/context";
import { useDateFormat } from "@/i18n/format";

const CUSTOM = "custom";

/** One segment of the date field: a number, or the punctuation between numbers. */
const segmentClass =
  "rounded px-0.5 tabular-nums outline-none type-literal:px-0 placeholder-shown:text-ink-faint " +
  "focus:bg-accent focus:text-accent-contrast focus:placeholder-shown:text-accent-contrast";

/**
 * The form that picks when a message is sent: one of the times offered outright (`quickTimes`)
 * or a date and time of the reader's own, on their clock. With `initial`, a time already chosen,
 * the form opens on the reader's own choice holding it, for it to be changed. `onSubmit` is
 * given the time and does what it is for (setting the message box to schedule what it holds,
 * giving a scheduled message another time); what it throws is shown here, and the form stays
 * for another try. With `onBack` it is a later step of its dialog.
 */
export function ScheduleTimeForm({
  heading,
  submitLabel,
  pendingLabel,
  initial,
  onSubmit,
  onBack,
}: {
  heading: string;
  submitLabel: string;
  pendingLabel: string;
  initial?: Date;
  onSubmit: (at: Date) => Promise<void>;
  onBack?: () => void;
}) {
  const m = useMessages();
  const shown = useDateFormat(SCHEDULED_TIME_FORMAT);
  // The times offered are those of the moment the form opened, so they do not shift under the
  // reader; what is chosen is checked against the clock again as it is submitted.
  const [opened] = useState(() => new Date());
  const [quick] = useState(() => quickTimes(opened));
  const [choice, setChoice] = useState<string>(
    initial === undefined ? (quick[0]?.key ?? CUSTOM) : CUSTOM,
  );
  const [custom, setCustom] = useState<CalendarDateTime | null>(() =>
    initial === undefined ? null : localDateTime(initial),
  );
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const chosen =
    choice === CUSTOM
      ? custom && toDate(custom)
      : (quick.find((time) => time.key === choice)?.at ?? null);

  async function submit() {
    if (chosen === null || pending) {
      return;
    }
    const problem = timeProblem(chosen, new Date());
    if (problem !== null) {
      setError(describeTimeProblem(m, problem));
      return;
    }
    setPending(true);
    setError(null);
    try {
      await onSubmit(chosen);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setPending(false);
    }
  }

  return (
    <Form
      // The form says what is wrong with a time itself, in the app's language, so the field's
      // bounds guide its picker without the browser refusing the submission in its own words.
      validationBehavior="aria"
      onSubmit={(event) => {
        event.preventDefault();
        void submit();
      }}
      className="flex flex-col gap-4"
    >
      <DialogHeading {...(onBack === undefined ? {} : { onBack })}>{heading}</DialogHeading>
      {error !== null && (
        <p role="alert" className={alertClass}>
          {error}
        </p>
      )}
      <RadioGroup value={choice} onChange={setChoice} className={fieldClass}>
        <Label className={labelClass}>{m.scheduled.when}</Label>
        <div className="flex flex-col gap-2">
          {quick.map((time) => (
            <RadioField key={time.key} value={time.key}>
              <RadioButton className={choiceClass}>
                <RadioMark />
                <span className="flex flex-col">
                  <span className="font-medium">{m.scheduled[time.key]}</span>
                  <span className="text-xs text-ink-muted">{shown.format(time.at)}</span>
                </span>
              </RadioButton>
            </RadioField>
          ))}
          <RadioField value={CUSTOM}>
            <RadioButton className={choiceClass}>
              <RadioMark />
              <span className="font-medium">{m.scheduled.custom}</span>
            </RadioButton>
          </RadioField>
        </div>
      </RadioGroup>
      {choice === CUSTOM && (
        // Typed segment by segment, in the app's language, on the reader's own clock. The bounds
        // mark what is out of range; `submit` says why.
        <DateField
          value={custom}
          onChange={setCustom}
          granularity="minute"
          minValue={localDateTime(new Date(opened.getTime() + MIN_AHEAD_MS))}
          maxValue={localDateTime(new Date(opened.getTime() + MAX_AHEAD_MS))}
          isRequired
          autoFocus
          className={fieldClass}
        >
          <Label className={labelClass}>{m.scheduled.customLabel}</Label>
          <DateInput className={inputClass + " flex"}>
            {(segment) => <DateSegment segment={segment} className={segmentClass} />}
          </DateInput>
        </DateField>
      )}
      <div className="flex justify-end gap-2">
        {/* Not disabled while pending, which `submit` ignores a second press during: a
            disabled button drops focus out of the dialog, which then no longer closes on
            Escape. */}
        <Button
          type="submit"
          isDisabled={chosen === null}
          aria-disabled={pending}
          className={primaryButtonClass + " aria-disabled:opacity-60"}
        >
          {pending ? pendingLabel : submitLabel}
        </Button>
      </div>
    </Form>
  );
}

/**
 * The schedule form in a modal, opened by the message box's schedule control. `onSchedule` is
 * given the time chosen, which the box then schedules its next message for (`ScheduleBar`), and
 * the modal closes. With `initial`, the time the box already holds, the form opens on it.
 */
export function ScheduleMessageModal({
  isOpen,
  onOpenChange,
  initial,
  onSchedule,
}: {
  isOpen: boolean;
  onOpenChange: (open: boolean) => void;
  initial?: Date;
  onSchedule: (at: Date) => void;
}) {
  const m = useMessages();
  return (
    <ModalOverlay
      isOpen={isOpen}
      onOpenChange={onOpenChange}
      className={overlayClass}
      isDismissable
    >
      <Modal className={modalClass}>
        <Dialog className={dialogClass}>
          {({ close }) => (
            <ScheduleTimeForm
              heading={m.scheduled.heading}
              submitLabel={m.scheduled.setTime}
              pendingLabel={m.scheduled.setTime}
              {...(initial === undefined ? {} : { initial })}
              onSubmit={(at) => {
                onSchedule(at);
                close();
                return Promise.resolve();
              }}
            />
          )}
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}
