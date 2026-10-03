import { ApiProblemError, type ProfileAspect, type ReportCategory } from "@aspen/protocol";
import { CaretDownIcon, FlagIcon } from "@phosphor-icons/react";
import { useEffect, useState } from "react";
import {
  Button,
  Dialog,
  DialogTrigger,
  Label,
  ListBox,
  ListBoxItem,
  Modal,
  ModalOverlay,
  Popover,
  Select,
  SelectValue,
  Text,
  TextArea,
  TextField,
  type ModalOverlayProps,
} from "react-aria-components";
import { useSync } from "@/api/hooks";
import {
  alertClass,
  fieldClass,
  hintClass,
  inputClass,
  labelClass,
  primaryButtonClass,
} from "@/features/auth/styles";
import {
  dialogClass,
  modalClass,
  optionClass,
  overlayClass,
  selectButtonClass,
  selectPopoverClass,
} from "@/features/invites/dialog";
import { ChoiceCheckbox } from "@/features/layout/choices";
import { DialogHeading } from "@/features/layout/DialogHeading";
import { Skeleton } from "@/features/layout/Skeleton";
import { toast } from "@/features/layout/toast";
import { Tooltip } from "@/features/layout/Tooltip";
import { ACTION_ICON } from "@/features/messages/actionIcon";
import { PROFILE_ASPECTS } from "@/features/users/profileAspects";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/** The longest explanation a report takes, as the server counts it. */
const EXPLANATION_MAX = 1000;

/** What is being reported: a message, or someone's profile. */
export type ReportTarget =
  { kind: "message"; messageId: string } | { kind: "profile"; userId: string; name: string };

/** A message's Report control and the dialog it opens. */
export function ReportMessageButton({
  messageId,
  triggerClassName,
}: {
  messageId: string;
  triggerClassName: string;
}) {
  const m = useMessages();
  return (
    <DialogTrigger>
      <Tooltip text={m.reports.reportMessage}>
        <Button className={triggerClassName} aria-label={m.reports.reportMessage}>
          <FlagIcon size={ACTION_ICON} aria-hidden="true" />
        </Button>
      </Tooltip>
      <ReportModal target={{ kind: "message", messageId }} />
    </DialogTrigger>
  );
}

/**
 * The report form, in a modal: opened by the trigger around it, or, given `isOpen`, by whatever
 * holds it (a touch screen's message actions, which close as it opens).
 */
export function ReportModal({
  target,
  ...overlay
}: { target: ReportTarget } & Omit<ModalOverlayProps, "children" | "className">) {
  return (
    <ModalOverlay {...overlay} className={overlayClass} isDismissable>
      <Modal className={modalClass}>
        <Dialog className={dialogClass}>
          {({ close }) => <ReportForm target={target} close={close} />}
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}

/**
 * Why the reporter is reporting: a category from those the server offers, each with what it
 * covers; for a profile, which of its aspects are wrong; and anything they would add, which
 * Other needs. Sent, it says so in a toast and closes.
 */
function ReportForm({ target, close }: { target: ReportTarget; close: () => void }) {
  const m = useMessages();
  const sync = useSync();
  const [categories, setCategories] = useState<readonly ReportCategory[] | null>(null);
  const [category, setCategory] = useState<string | null>(null);
  const [aspects, setAspects] = useState<readonly ProfileAspect[]>([]);
  const [explanation, setExplanation] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let current = true;
    sync.reportCategories().then(
      (found) => {
        if (current) {
          setCategories(found);
        }
      },
      (e: unknown) => {
        if (current) {
          setError(e instanceof ApiProblemError ? e.message : String(e));
        }
      },
    );
    return () => {
      current = false;
    };
  }, [sync]);

  const chosen = categories?.find((c) => c.id === category);
  const explanationRequired = chosen?.builtin === "other";
  const ready =
    chosen !== undefined &&
    (target.kind === "message" || aspects.length > 0) &&
    (!explanationRequired || explanation.trim() !== "");

  async function send() {
    if (!ready || pending || category === null) {
      return;
    }
    setPending(true);
    setError(null);
    const said = explanation.trim() === "" ? null : explanation.trim();
    try {
      if (target.kind === "message") {
        await sync.reportMessage(target.messageId, category, said);
      } else {
        await sync.reportProfile(target.userId, category, said, aspects);
      }
      toast(m.reports.sent);
      close();
    } catch (e) {
      setError(e instanceof ApiProblemError ? e.message : String(e));
      setPending(false);
    }
  }

  return (
    <form
      className="flex flex-col gap-4"
      onSubmit={(e) => {
        e.preventDefault();
        void send();
      }}
    >
      <DialogHeading>
        {target.kind === "message"
          ? m.reports.reportMessageHeading
          : format(m.reports.reportProfileHeading, { name: target.name })}
      </DialogHeading>
      <p className="text-sm text-ink-muted">
        {target.kind === "message"
          ? m.reports.reportMessageHint
          : format(m.reports.reportProfileHint, { name: target.name })}
      </p>
      {target.kind === "profile" && (
        <fieldset className="flex flex-col gap-2">
          <legend className={labelClass + " mb-1"}>{m.reports.aspectsLabel}</legend>
          <div className="grid grid-cols-2 gap-2">
            {PROFILE_ASPECTS.map((aspect) => (
              <ChoiceCheckbox
                key={aspect}
                isSelected={aspects.includes(aspect)}
                onChange={(selected) => {
                  setAspects((now) =>
                    selected ? [...now, aspect] : now.filter((a) => a !== aspect),
                  );
                }}
                label={m.reports.aspects[aspect]}
              />
            ))}
          </div>
        </fieldset>
      )}
      {categories === null ? (
        <div aria-busy="true" className="flex flex-col gap-1">
          <span className={labelClass}>{m.reports.categoryLabel}</span>
          <Skeleton className="h-9 w-full" />
          <span className="sr-only">{m.reports.categoriesLoading}</span>
        </div>
      ) : (
        <Select
          value={category}
          onChange={(key) => {
            setCategory(typeof key === "string" ? key : null);
          }}
          placeholder={m.reports.categoryPlaceholder}
          className="flex flex-col gap-1"
        >
          <Label className={labelClass}>{m.reports.categoryLabel}</Label>
          <Button className={selectButtonClass}>
            <SelectValue className="truncate">
              {({ isPlaceholder, selectedText, defaultChildren }) =>
                isPlaceholder ? defaultChildren : selectedText
              }
            </SelectValue>
            <CaretDownIcon size={14} aria-hidden="true" className="shrink-0 text-ink-faint" />
          </Button>
          {chosen?.description != null && <p className={hintClass}>{chosen.description}</p>}
          <Popover className={selectPopoverClass + " max-h-80 overflow-y-auto"}>
            <ListBox>
              {categories.map((c) => (
                <ListBoxItem
                  key={c.id}
                  id={c.id}
                  textValue={c.name}
                  className={optionClass + " flex flex-col"}
                >
                  <Text slot="label">{c.name}</Text>
                  {c.description != null && (
                    <Text slot="description" className="text-xs font-normal text-ink-muted">
                      {c.description}
                    </Text>
                  )}
                </ListBoxItem>
              ))}
            </ListBox>
          </Popover>
        </Select>
      )}
      <TextField
        value={explanation}
        onChange={setExplanation}
        maxLength={EXPLANATION_MAX}
        isRequired={explanationRequired}
        className={fieldClass}
      >
        <Label className={labelClass}>
          {explanationRequired ? m.reports.explanationRequiredLabel : m.reports.explanationLabel}
        </Label>
        <TextArea rows={3} className={inputClass + " max-h-48 resize-none field-sizing-content"} />
        <p className={hintClass}>{m.reports.explanationHint}</p>
      </TextField>
      {error !== null && (
        <p role="alert" className={alertClass}>
          {error}
        </p>
      )}
      <div className="flex justify-end">
        <Button type="submit" isDisabled={!ready || pending} className={primaryButtonClass}>
          {pending ? m.reports.sending : m.reports.send}
        </Button>
      </div>
    </form>
  );
}
