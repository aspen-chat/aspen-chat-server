import { ApiProblemError, type Poll } from "@aspen/protocol";
import { CheckIcon, XIcon } from "@phosphor-icons/react";
import { useEffect, useState } from "react";
import {
  Button,
  Dialog,
  DialogTrigger,
  Form,
  Input,
  Modal,
  ModalOverlay,
  TextField,
  ToggleButton,
} from "react-aria-components";
import {
  useMe,
  useMyVotes,
  useMyWriteIns,
  usePoll,
  useStore,
  useSync,
  useChannelCan,
} from "@/api/hooks";
import { inputClass } from "@/features/auth/styles";
import {
  dangerButtonClass,
  dialogClass,
  modalClass,
  overlayClass,
  secondaryButtonClass,
} from "@/features/invites/dialog";
import { Tooltip } from "@/features/layout/Tooltip";
import {
  MAX_OPTION_CHARS,
  MAX_WRITE_INS,
  optionName,
  pollChoices,
  pollOpen,
  remainingText,
  sharePercent,
  type PollChoice,
} from "@/features/messages/poll";
import { displayNameOf } from "@/features/users/profile";
import { DialogHeading } from "@/features/layout/DialogHeading";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/**
 * A poll in its message: the question, one bar per answer showing its share of the votes, the
 * caller's own choices, and how long voting stays open. Voting is a toggle on each answer
 * while the poll is open; the tally itself updates from the server's events. A poll that takes
 * write-ins lists them after its creator's options, each noting who wrote it in, and offers a
 * field to add one to anyone who has not already.
 */
export function PollCard({ pollId }: { pollId: string }) {
  const m = useMessages();
  const poll = usePoll(pollId);
  if (poll === undefined) {
    return <p className="mt-1 text-sm text-ink-faint">{m.poll.unavailable}</p>;
  }
  return <LoadedPollCard poll={poll} />;
}

function LoadedPollCard({ poll }: { poll: Poll }) {
  const m = useMessages();
  const myWriteIns = useMyWriteIns(poll.id);
  const now = useNow(poll);
  const open = pollOpen(poll, now);
  const total = poll.results.reduce((sum, r) => sum + r.count, 0);
  const canWriteIn =
    open && poll.allowWriteIns && myWriteIns.size === 0 && poll.writeIns.length < MAX_WRITE_INS;

  return (
    <section
      aria-label={format(m.poll.label, { question: poll.question })}
      className="mt-1 flex w-full max-w-lg flex-col gap-2 rounded-md border border-line bg-surface-raised p-3"
    >
      <h3 className="font-medium">{poll.question}</h3>
      <ul className="flex flex-col gap-1.5">
        {pollChoices(poll).map((choice) => (
          <ChoiceRow key={choice.index} poll={poll} choice={choice} open={open} />
        ))}
      </ul>
      {canWriteIn && <WriteInField pollId={poll.id} />}
      <p className="flex flex-wrap gap-x-3 text-xs text-ink-faint">
        <span>
          {total === 1 ? m.poll.voteSingular : format(m.poll.votesTotal, { count: String(total) })}
        </span>
        {poll.multipleChoice && <span>{m.poll.multipleTag}</span>}
        {poll.anonymous && <span>{m.poll.anonymousTag}</span>}
        <span>
          {open
            ? format(m.poll.closesIn, { remaining: remainingText(m, poll.closesAt, now) })
            : m.poll.closed}
        </span>
      </p>
    </section>
  );
}

/**
 * One answer: a toggle that votes for it, drawn as a bar of its share, and, on a write-in the
 * caller wrote or on any write-in of a poll the caller made, a control that removes it.
 */
function ChoiceRow({ poll, choice, open }: { poll: Poll; choice: PollChoice; open: boolean }) {
  const m = useMessages();
  const sync = useSync();
  const store = useStore();
  const me = useMe();
  const myVotes = useMyVotes(poll.id);
  const myWriteIns = useMyWriteIns(poll.id);
  const moderate = useChannelCan(poll.channelId, "manageMessages");
  const { index, option } = choice;
  const mine = myVotes.has(index);
  const voters = poll.results[index]?.voters ?? null;
  const nameOf = (id: string) => {
    const user = store.user(id);
    return user === undefined ? m.unknownUser : displayNameOf(user);
  };
  const names = voters === null ? null : voters.map(nameOf).join(", ");
  const note = !choice.writeIn
    ? null
    : choice.writtenBy === null
      ? m.poll.writtenIn
      : format(m.poll.writtenInBy, { name: nameOf(choice.writtenBy) });
  // A write-in can be removed by its writer, the poll's creator, and anyone who may manage
  // messages here.
  const canRemove =
    open &&
    choice.writeIn &&
    (moderate ||
      myWriteIns.has(index) ||
      (me !== null && (choice.writtenBy === me.id || poll.createdBy === me.id)));

  function toggle(selected: boolean) {
    void (selected ? sync.vote(poll.id, index) : sync.unvote(poll.id, index)).catch(
      () => undefined,
    );
  }

  return (
    <li className="flex items-center gap-1">
      <ToggleButton
        isSelected={mine}
        isDisabled={!open}
        onChange={toggle}
        aria-label={format(mine ? m.poll.unvote : m.poll.vote, { option: optionName(option) })}
        className={
          "relative flex min-w-0 flex-1 items-center gap-2 overflow-hidden rounded-md border px-3 py-1.5 text-left text-sm outline-none " +
          "focus-visible:ring-2 focus-visible:ring-accent/50 " +
          (mine ? "border-accent" : "border-line") +
          (open ? " hover:bg-surface-hover pressed:opacity-80" : " cursor-default")
        }
      >
        <span
          aria-hidden="true"
          className={
            "absolute inset-y-0 left-0 transition-[width] " +
            (mine ? "bg-accent-soft" : "bg-surface-sunken")
          }
          style={{ width: `${String(sharePercent(poll, index))}%` }}
        />
        <span className="relative flex min-w-0 flex-1 flex-col">
          <span className="flex items-center gap-1.5">
            {mine && <CheckIcon size={14} weight="bold" aria-hidden="true" />}
            {option.emoji != null && (
              <span className="text-base leading-none" aria-hidden="true">
                {option.emoji}
              </span>
            )}
            <span className="truncate">{option.label}</span>
          </span>
          {note !== null && <span className="truncate text-xs text-ink-faint">{note}</span>}
          {names !== null && names.length > 0 && (
            <span className="truncate text-xs text-ink-muted">
              {format(m.poll.votedBy, { names })}
            </span>
          )}
        </span>
        <span className="relative tabular-nums text-ink-muted">
          {String(sharePercent(poll, index))}%
        </span>
      </ToggleButton>
      {canRemove && <RemoveWriteInDialog pollId={poll.id} index={index} label={option.label} />}
    </li>
  );
}

/**
 * The field that writes in the caller's own answer. An answer the poll already has, however
 * it is capitalised or spaced, is voted for instead of added.
 */
function WriteInField({ pollId }: { pollId: string }) {
  const m = useMessages();
  const sync = useSync();
  const [label, setLabel] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const trimmed = label.trim();

  async function submit() {
    if (pending || trimmed.length === 0) {
      return;
    }
    setPending(true);
    setError(null);
    try {
      await sync.writeIn(pollId, trimmed);
      setLabel("");
    } catch (e) {
      setError(e instanceof ApiProblemError ? e.message : String(e));
    }
    setPending(false);
  }

  return (
    <Form
      onSubmit={(event) => {
        event.preventDefault();
        void submit();
      }}
      className="flex flex-col gap-1"
    >
      <div className="flex items-center gap-1.5">
        <TextField
          value={label}
          onChange={setLabel}
          maxLength={MAX_OPTION_CHARS}
          aria-label={m.poll.writeIn}
          className="min-w-0 flex-1"
        >
          <Input placeholder={m.poll.writeIn} className={inputClass + " w-full"} />
        </TextField>
        <Button
          type="submit"
          isDisabled={pending || trimmed.length === 0}
          className={secondaryButtonClass + " shrink-0 self-stretch"}
        >
          {pending ? m.poll.writeInAdding : m.poll.writeInSubmit}
        </Button>
      </div>
      {error !== null && (
        <p role="alert" className="text-xs text-danger">
          {error}
        </p>
      )}
    </Form>
  );
}

/** A write-in's remove control and the confirmation it asks for, since its votes go with it. */
function RemoveWriteInDialog({
  pollId,
  index,
  label,
}: {
  pollId: string;
  index: number;
  label: string;
}) {
  const m = useMessages();
  const name = format(m.poll.removeWriteIn, { option: label });
  return (
    <DialogTrigger>
      <Tooltip text={name}>
        <Button
          aria-label={name}
          className={
            "tap-target shrink-0 rounded-md p-1.5 text-ink-muted outline-none hover:bg-surface-hover hover:text-ink " +
            "pressed:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50"
          }
        >
          <XIcon size={16} aria-hidden="true" />
        </Button>
      </Tooltip>
      <ModalOverlay className={overlayClass} isDismissable>
        <Modal className={modalClass}>
          <Dialog role="alertdialog" className={dialogClass}>
            {({ close }) => (
              <ConfirmRemove pollId={pollId} index={index} label={label} close={close} />
            )}
          </Dialog>
        </Modal>
      </ModalOverlay>
    </DialogTrigger>
  );
}

function ConfirmRemove({
  pollId,
  index,
  label,
  close,
}: {
  pollId: string;
  index: number;
  label: string;
  close: () => void;
}) {
  const m = useMessages();
  const sync = useSync();
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function confirm() {
    setPending(true);
    setError(null);
    try {
      await sync.removeWriteIn(pollId, index);
      close();
    } catch (e) {
      setError(e instanceof ApiProblemError ? e.message : String(e));
      setPending(false);
    }
  }

  return (
    <>
      <DialogHeading>{m.poll.removeWriteInHeading}</DialogHeading>
      <p className="text-sm text-ink-muted">
        {format(m.poll.removeWriteInHint, { option: label })}
      </p>
      {error !== null && (
        <p role="alert" className="text-sm text-danger">
          {error}
        </p>
      )}
      <div className="flex justify-end gap-2">
        <Button
          isDisabled={pending}
          onPress={() => {
            void confirm();
          }}
          className={dangerButtonClass}
        >
          {pending ? m.poll.removing : m.poll.remove}
        </Button>
      </div>
    </>
  );
}

/**
 * The current time, refreshed while the poll is open: every second in its last hour, when
 * seconds are shown, and every minute before that.
 */
function useNow(poll: Poll): number {
  const [now, setNow] = useState(() => Date.now());
  const open = pollOpen(poll, now);
  const lastHour = Date.parse(poll.closesAt) - now < 3_600_000;
  useEffect(() => {
    if (!open) {
      return;
    }
    const timer = setInterval(
      () => {
        setNow(Date.now());
      },
      lastHour ? 1000 : 60_000,
    );
    return () => {
      clearInterval(timer);
    };
  }, [open, lastHour]);
  return now;
}
