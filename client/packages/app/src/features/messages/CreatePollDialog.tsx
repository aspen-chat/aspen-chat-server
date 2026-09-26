import { ApiProblemError, type PollOption } from "@aspen/protocol";
import {
  CaretDownIcon,
  ChartBarIcon,
  CheckIcon,
  PlusIcon,
  SmileyIcon,
  XIcon,
} from "@phosphor-icons/react";
import { lazy, Suspense, useState } from "react";
import {
  Button,
  CheckboxButton,
  CheckboxField,
  Dialog,
  DialogTrigger,
  Form,
  Heading,
  Input,
  Label,
  ListBox,
  ListBoxItem,
  Modal,
  ModalOverlay,
  Popover,
  RadioButton,
  RadioField,
  RadioGroup,
  Select,
  SelectValue,
  TextField,
} from "react-aria-components";
import { useSync } from "@/api/hooks";
import { Tooltip } from "@/features/layout/Tooltip";
import {
  alertClass,
  fieldClass,
  inputClass,
  labelClass,
  primaryButtonClass,
} from "@/features/auth/styles";
import {
  dialogClass,
  headingClass,
  modalClass,
  overlayClass,
  secondaryButtonClass,
} from "@/features/invites/dialog";
import { useMessages } from "@/i18n/context";
import { format, type Messages } from "@/i18n/messages";

/** The emoji picker is a sizeable chunk, fetched the first time anyone opens it. */
const EmojiPicker = lazy(() => import("@/features/messages/EmojiPicker"));

/** Bounds mirrored from the server's `app::poll`, so the form refuses what it would refuse. */
const MIN_OPTIONS = 2;
const MAX_OPTIONS = 10;
const MAX_QUESTION_CHARS = 300;
const MAX_OPTION_CHARS = 100;

const DURATIONS: readonly { key: keyof Messages["poll"]["durations"]; seconds: number }[] = [
  { key: "fiveMinutes", seconds: 5 * 60 },
  { key: "hour", seconds: 60 * 60 },
  { key: "fourHours", seconds: 4 * 60 * 60 },
  { key: "day", seconds: 24 * 60 * 60 },
  { key: "threeDays", seconds: 3 * 24 * 60 * 60 },
  { key: "week", seconds: 7 * 24 * 60 * 60 },
];

const selectButtonClass =
  "flex justify-between rounded-md border border-line bg-surface px-3 py-2 text-left outline-none " +
  "hover:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50";
const popoverClass =
  "min-w-(--trigger-width) rounded-md border border-line bg-surface-raised p-1 shadow-lg";
const listOptionClass =
  "cursor-default rounded px-2 py-1 text-sm outline-none focus:bg-surface-hover selected:font-medium selected:text-accent";
const radioClass =
  "group flex items-start gap-2 rounded-md border border-line px-3 py-2 text-sm outline-none " +
  "hover:bg-surface-hover selected:border-accent focus-visible:ring-2 focus-visible:ring-accent/50";
const boxClass =
  "mt-0.5 flex h-4 w-4 shrink-0 items-center justify-center rounded border border-line bg-surface " +
  "text-accent-contrast group-selected:border-accent group-selected:bg-accent";
const iconButtonClass =
  "rounded-md p-1.5 text-ink-muted outline-none hover:bg-surface-hover hover:text-ink " +
  "pressed:bg-surface-hover disabled:opacity-40 focus-visible:ring-2 focus-visible:ring-accent/50";

/** The composer's poll control and the form it opens. */
export function CreatePollDialog({
  channelId,
  triggerClassName,
}: {
  channelId: string;
  triggerClassName: string;
}) {
  const m = useMessages();
  return (
    <DialogTrigger>
      <Tooltip text={m.poll.open}>
        <Button aria-label={m.poll.open} className={triggerClassName}>
          <ChartBarIcon size={20} aria-hidden="true" />
        </Button>
      </Tooltip>
      <ModalOverlay className={overlayClass} isDismissable>
        <Modal className={modalClass}>
          <Dialog className={dialogClass}>
            {({ close }) => <PollForm channelId={channelId} close={close} />}
          </Dialog>
        </Modal>
      </ModalOverlay>
    </DialogTrigger>
  );
}

function PollForm({ channelId, close }: { channelId: string; close: () => void }) {
  const m = useMessages();
  const sync = useSync();
  const [question, setQuestion] = useState("");
  const [options, setOptions] = useState<PollOption[]>([{ label: "" }, { label: "" }]);
  const [multipleChoice, setMultipleChoice] = useState(false);
  const [anonymous, setAnonymous] = useState(false);
  const [duration, setDuration] = useState<string>("day");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const trimmedOptions = options.flatMap((o) => {
    const label = o.label.trim();
    return label.length === 0 ? [] : [o.emoji == null ? { label } : { label, emoji: o.emoji }];
  });
  const canSubmit = !pending && question.trim().length > 0 && trimmedOptions.length >= MIN_OPTIONS;

  function setOption(index: number, patch: Partial<PollOption>) {
    setOptions((list) => list.map((o, i) => (i === index ? { ...o, ...patch } : o)));
  }

  async function submit() {
    const seconds = DURATIONS.find((d) => d.key === duration)?.seconds;
    if (!canSubmit || seconds === undefined) {
      return;
    }
    setPending(true);
    setError(null);
    try {
      await sync.createPoll(channelId, {
        question: question.trim(),
        options: trimmedOptions,
        multipleChoice,
        anonymous,
        durationSeconds: seconds,
      });
      close();
    } catch (e) {
      setError(e instanceof ApiProblemError ? e.message : String(e));
      setPending(false);
    }
  }

  return (
    <Form
      onSubmit={(event) => {
        event.preventDefault();
        void submit();
      }}
      className="flex flex-col gap-4"
    >
      <Heading slot="title" className={headingClass}>
        {m.poll.heading}
      </Heading>
      {error !== null && (
        <p role="alert" className={alertClass}>
          {error}
        </p>
      )}
      <TextField
        value={question}
        onChange={setQuestion}
        maxLength={MAX_QUESTION_CHARS}
        isRequired
        autoFocus
        className={fieldClass}
      >
        <Label className={labelClass}>{m.poll.question}</Label>
        <Input placeholder={m.poll.questionPlaceholder} className={inputClass} />
      </TextField>
      <fieldset className={fieldClass}>
        <legend className={labelClass}>{m.poll.options}</legend>
        <ul className="flex flex-col gap-1.5">
          {options.map((option, index) => (
            <li key={index} className="flex items-center gap-1">
              <OptionEmojiPicker
                index={index}
                emoji={option.emoji ?? null}
                onChange={(emoji) => {
                  setOption(index, { emoji });
                }}
              />
              <TextField
                value={option.label}
                onChange={(label) => {
                  setOption(index, { label });
                }}
                maxLength={MAX_OPTION_CHARS}
                aria-label={format(m.poll.optionPlaceholder, { n: String(index + 1) })}
                className="flex-1"
              >
                <Input
                  placeholder={format(m.poll.optionPlaceholder, { n: String(index + 1) })}
                  className={inputClass + " w-full"}
                />
              </TextField>
              <Button
                aria-label={format(m.poll.removeOption, { n: String(index + 1) })}
                isDisabled={options.length <= MIN_OPTIONS}
                onPress={() => {
                  setOptions((list) => list.filter((_, i) => i !== index));
                }}
                className={iconButtonClass}
              >
                <XIcon size={16} aria-hidden="true" />
              </Button>
            </li>
          ))}
        </ul>
        <Button
          isDisabled={options.length >= MAX_OPTIONS}
          onPress={() => {
            setOptions((list) => [...list, { label: "" }]);
          }}
          className={secondaryButtonClass + " flex items-center gap-1 self-start"}
        >
          <PlusIcon size={14} aria-hidden="true" />
          {m.poll.addOption}
        </Button>
      </fieldset>
      <RadioGroup
        value={multipleChoice ? "multiple" : "single"}
        onChange={(value) => {
          setMultipleChoice(value === "multiple");
        }}
        className={fieldClass}
      >
        <Label className={labelClass}>{m.poll.choices}</Label>
        <div className="grid grid-cols-2 gap-2">
          <RadioField value="single">
            <RadioButton className={radioClass}>
              <RadioMark />
              <span className="flex flex-col">
                <span className="font-medium">{m.poll.singleChoice}</span>
                <span className="text-xs text-ink-muted">{m.poll.singleChoiceHint}</span>
              </span>
            </RadioButton>
          </RadioField>
          <RadioField value="multiple">
            <RadioButton className={radioClass}>
              <RadioMark />
              <span className="flex flex-col">
                <span className="font-medium">{m.poll.multipleChoice}</span>
                <span className="text-xs text-ink-muted">{m.poll.multipleChoiceHint}</span>
              </span>
            </RadioButton>
          </RadioField>
        </div>
      </RadioGroup>
      <Select
        value={duration}
        onChange={(key) => {
          setDuration(String(key));
        }}
        className={fieldClass}
      >
        <Label className={labelClass}>{m.poll.duration}</Label>
        <Button className={selectButtonClass}>
          <SelectValue />
          <CaretDownIcon size={16} aria-hidden="true" className="text-ink-muted" />
        </Button>
        <Popover className={popoverClass}>
          <ListBox>
            {DURATIONS.map((d) => (
              <ListBoxItem key={d.key} id={d.key} className={listOptionClass}>
                {m.poll.durations[d.key]}
              </ListBoxItem>
            ))}
          </ListBox>
        </Popover>
      </Select>
      <CheckboxField isSelected={anonymous} onChange={setAnonymous}>
        <CheckboxButton className={radioClass}>
          <span className={boxClass}>
            <CheckIcon
              size={12}
              weight="bold"
              aria-hidden="true"
              className="hidden group-selected:block"
            />
          </span>
          <span className="flex flex-col">
            <span className="font-medium">{m.poll.anonymous}</span>
            <span className="text-xs text-ink-muted">{m.poll.anonymousHint}</span>
          </span>
        </CheckboxButton>
      </CheckboxField>
      <div className="flex justify-end gap-2">
        <Button onPress={close} className={secondaryButtonClass}>
          {m.cancel}
        </Button>
        <Button type="submit" isDisabled={!canSubmit} className={primaryButtonClass}>
          {pending ? m.poll.creating : m.poll.create}
        </Button>
      </div>
    </Form>
  );
}

/**
 * The emoji control before an option's text: shows the chosen emoji, or a smiley when there is
 * none, and opens the picker. Picking the same emoji again clears it.
 */
function OptionEmojiPicker({
  index,
  emoji,
  onChange,
}: {
  index: number;
  emoji: string | null;
  onChange: (emoji: string | null) => void;
}) {
  const m = useMessages();
  const label = format(emoji === null ? m.poll.pickEmoji : m.poll.changeEmoji, {
    n: String(index + 1),
  });
  return (
    <DialogTrigger>
      <Button aria-label={label} className={iconButtonClass + " text-base leading-none"}>
        {emoji ?? <SmileyIcon size={18} aria-hidden="true" />}
      </Button>
      <Popover
        placement="bottom start"
        className="rounded-lg border border-line bg-surface-raised shadow-lg"
      >
        <Dialog aria-label={label} className="outline-none">
          {({ close }) => (
            <div className="flex flex-col">
              <Suspense
                fallback={
                  <div className="flex h-96 w-80 items-center justify-center text-sm text-ink-muted">
                    {m.loading}
                  </div>
                }
              >
                <EmojiPicker
                  onPick={(picked) => {
                    onChange(picked === emoji ? null : picked);
                    close();
                  }}
                />
              </Suspense>
              {emoji !== null && (
                <Button
                  onPress={() => {
                    onChange(null);
                    close();
                  }}
                  className={secondaryButtonClass + " m-2"}
                >
                  {m.poll.clearEmoji}
                </Button>
              )}
            </div>
          )}
        </Dialog>
      </Popover>
    </DialogTrigger>
  );
}

function RadioMark() {
  return (
    <span className={boxClass + " rounded-full"}>
      <span className="h-1.5 w-1.5 rounded-full bg-accent-contrast opacity-0 group-selected:opacity-100" />
    </span>
  );
}
