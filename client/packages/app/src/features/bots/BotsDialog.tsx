import { ALL_PERMISSIONS, ApiProblemError, type Permission, type User } from "@aspen/protocol";
import { CopyIcon, PlusIcon, RobotIcon } from "@phosphor-icons/react";
import { useEffect, useRef, useState } from "react";
import {
  Button,
  Dialog,
  DialogTrigger,
  Input,
  Label,
  Modal,
  ModalOverlay,
  TextField,
} from "react-aria-components";
import { useMe, useOwnedBots, useSync } from "@/api/hooks";
import { fieldClass, inputClass, labelClass, primaryButtonClass } from "@/features/auth/styles";
import { botAddLink } from "@/features/bots/botLink";
import { Avatar } from "@/features/communities/Avatar";
import { PermissionChecklist } from "@/features/community-settings/PermissionChecklist";
import { PeoplePicker } from "@/features/dms/PeoplePicker";
import {
  dangerButtonClass,
  dialogClass,
  overlayClass,
  secondaryButtonClass,
  wideModalClass,
} from "@/features/invites/dialog";
import { copyText } from "@/features/layout/clipboard";
import { ChoiceCheckbox } from "@/features/layout/choices";
import { DialogHeading } from "@/features/layout/DialogHeading";
import { displayNameOf } from "@/features/users/profile";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

const EVERY_PERMISSION: ReadonlySet<Permission> = new Set(ALL_PERMISSIONS);

function problemText(e: unknown): string {
  return e instanceof ApiProblemError ? e.message : String(e);
}

/**
 * The bots the user owns, opened from developer mode in Settings: making one, and for each,
 * whether it is public, the link that adds it with the permissions it suggests, a new token,
 * handing it to someone, and deleting it. A token is shown once, when it is issued.
 */
export function BotsDialog() {
  const m = useMessages();
  return (
    <DialogTrigger>
      <Button className={secondaryButtonClass + " flex items-center gap-1.5 self-start"}>
        <RobotIcon size={16} aria-hidden="true" />
        {m.bots.manage}
      </Button>
      <ModalOverlay isDismissable className={overlayClass}>
        <Modal className={wideModalClass}>
          <Dialog className={dialogClass}>
            <DialogHeading>{m.bots.heading}</DialogHeading>
            <BotsBody />
          </Dialog>
        </Modal>
      </ModalOverlay>
    </DialogTrigger>
  );
}

function BotsBody() {
  const m = useMessages();
  const sync = useSync();
  const bots = useOwnedBots();
  const [selected, setSelected] = useState<string | null>(null);
  const [token, setToken] = useState<{ botId: string; token: string } | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    sync.loadBots().catch((e: unknown) => {
      setError(problemText(e));
    });
  }, [sync]);
  const current = bots.find((bot) => bot.id === selected) ?? bots[0];
  return (
    <div className="flex flex-col gap-4 md:flex-row">
      <div className="flex flex-col gap-3 md:w-60 md:shrink-0">
        {bots.length === 0 ? (
          <p className="text-sm text-ink-muted">{m.bots.none}</p>
        ) : (
          <ul aria-label={m.bots.heading} className="flex flex-col gap-0.5">
            {bots.map((bot) => (
              <li key={bot.id}>
                <Button
                  aria-pressed={bot.id === current?.id}
                  onPress={() => {
                    setSelected(bot.id);
                  }}
                  className="flex w-full items-center gap-2 rounded-md px-2 py-1 text-start text-sm outline-none hover:bg-surface-hover pressed:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50 aria-pressed:bg-accent-soft aria-pressed:text-accent-strong"
                >
                  <Avatar name={displayNameOf(bot)} iconId={bot.icon} size="sm" />
                  <span className="min-w-0 flex-1 truncate">{displayNameOf(bot)}</span>
                </Button>
              </li>
            ))}
          </ul>
        )}
        <CreateBot
          onCreated={(bot, issued) => {
            setSelected(bot.id);
            setToken({ botId: bot.id, token: issued });
          }}
        />
        {error !== null && (
          <p role="alert" className="text-sm text-danger">
            {error}
          </p>
        )}
      </div>
      {current !== undefined && (
        <BotEditor
          key={current.id}
          bot={current}
          token={token?.botId === current.id ? token.token : null}
          onToken={(issued) => {
            setToken({ botId: current.id, token: issued });
          }}
          onGone={() => {
            setSelected(null);
            setToken(null);
          }}
        />
      )}
    </div>
  );
}

function CreateBot({ onCreated }: { onCreated: (bot: User, token: string) => void }) {
  const m = useMessages();
  const sync = useSync();
  const [name, setName] = useState("");
  const [displayName, setDisplayName] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  return (
    <form
      onSubmit={(event) => {
        event.preventDefault();
        setPending(true);
        setError(null);
        sync.createBot(name.trim(), displayName.trim() === "" ? null : displayName.trim()).then(
          ({ bot, token }) => {
            setName("");
            setDisplayName("");
            setPending(false);
            onCreated(bot, token);
          },
          (e: unknown) => {
            setError(problemText(e));
            setPending(false);
          },
        );
      }}
      className="flex flex-col gap-2 border-t border-line pt-3"
    >
      <h3 className="text-sm font-semibold text-ink-muted">{m.bots.create}</h3>
      <TextField value={name} onChange={setName} isRequired maxLength={32} className={fieldClass}>
        <Label className={labelClass}>{m.bots.nameLabel}</Label>
        <Input className={inputClass} />
      </TextField>
      <TextField
        value={displayName}
        onChange={setDisplayName}
        maxLength={32}
        className={fieldClass}
      >
        <Label className={labelClass}>{m.bots.displayNameLabel}</Label>
        <Input className={inputClass} />
      </TextField>
      {error !== null && (
        <p role="alert" className="text-sm text-danger">
          {error}
        </p>
      )}
      <Button
        type="submit"
        isDisabled={pending || name.trim() === ""}
        className={primaryButtonClass + " flex items-center justify-center gap-1.5"}
      >
        <PlusIcon size={14} aria-hidden="true" />
        {pending ? m.bots.making : m.bots.make}
      </Button>
    </form>
  );
}

function BotEditor({
  bot,
  token,
  onToken,
  onGone,
}: {
  bot: User;
  token: string | null;
  onToken: (token: string) => void;
  onGone: () => void;
}) {
  const m = useMessages();
  const sync = useSync();
  const me = useMe();
  const name = displayNameOf(bot);
  const [suggested, setSuggested] = useState<ReadonlySet<Permission>>(new Set());
  const [confirming, setConfirming] = useState<"token" | "delete" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const link = botAddLink(
    bot.id,
    ALL_PERMISSIONS.filter((p) => suggested.has(p)),
  );
  const run = (action: Promise<unknown>, after?: () => void) => {
    setError(null);
    action.then(
      () => {
        setConfirming(null);
        after?.();
      },
      (e: unknown) => {
        setError(problemText(e));
      },
    );
  };
  return (
    <section aria-label={name} className="flex min-w-0 flex-1 flex-col gap-4">
      <div className="flex items-center gap-3">
        <Avatar name={name} iconId={bot.icon} size="lg" />
        <div className="min-w-0">
          <div className="truncate text-base font-semibold">{name}</div>
          <div className="truncate text-sm text-ink-muted">@{bot.name}</div>
        </div>
      </div>
      {token !== null && <TokenReveal name={name} token={token} />}
      <ChoiceCheckbox
        isSelected={bot.botPublic}
        onChange={(isPublic) => {
          run(sync.setBotPublic(bot.id, isPublic));
        }}
        label={m.bots.public}
        hint={m.bots.publicHint}
      />
      <section className="flex flex-col gap-2">
        <h3 className="text-sm font-semibold text-ink-muted">{m.bots.addLink}</h3>
        <CopyField label={m.bots.addLink} value={link} copyLabel={m.bots.copyLink} />
        <details className="text-sm">
          <summary className="cursor-pointer text-ink-muted">{m.bots.addLinkHint}</summary>
          <div className="pt-3">
            <PermissionChecklist
              value={suggested}
              onChange={setSuggested}
              held={EVERY_PERMISSION}
            />
          </div>
        </details>
      </section>
      {confirming === "token" && <p className="text-sm">{m.bots.newTokenConfirm}</p>}
      {confirming === "delete" && (
        <p className="text-sm">{format(m.bots.deleteConfirm, { name })}</p>
      )}
      {error !== null && (
        <p role="alert" className="text-sm text-danger">
          {error}
        </p>
      )}
      <div className="flex flex-wrap gap-2">
        <Button
          onPress={() => {
            if (confirming !== "token") {
              setConfirming("token");
              return;
            }
            run(
              sync.rotateBotToken(bot.id).then((issued) => {
                onToken(issued);
              }),
            );
          }}
          className={confirming === "token" ? dangerButtonClass : secondaryButtonClass}
        >
          {m.bots.newToken}
        </Button>
        <PeoplePicker
          trigger={<Button className={secondaryButtonClass}>{m.bots.transfer}</Button>}
          heading={format(m.bots.transferHeading, { name })}
          confirmLabel={m.bots.transfer}
          pendingLabel={m.bots.transfer}
          exclude={[bot.id, ...(me === null ? [] : [me.id])]}
          max={1}
          onConfirm={async ([owner]) => {
            if (owner !== undefined) {
              await sync.transferBot(bot.id, owner);
              onGone();
            }
          }}
        />
        <Button
          onPress={() => {
            if (confirming !== "delete") {
              setConfirming("delete");
              return;
            }
            run(sync.deleteBot(bot.id), onGone);
          }}
          className={
            (confirming === "delete" ? dangerButtonClass : secondaryButtonClass) +
            (confirming === "delete" ? "" : " text-danger")
          }
        >
          {m.bots.delete}
        </Button>
      </div>
    </section>
  );
}

/** A freshly issued token, shown this once, with a way to copy it. */
function TokenReveal({ name, token }: { name: string; token: string }) {
  const m = useMessages();
  return (
    <section className="flex flex-col gap-2 rounded-md border border-accent/40 bg-accent-soft/40 p-3">
      <h3 className="text-sm font-semibold">{format(m.bots.tokenHeading, { name })}</h3>
      <p className="text-xs text-ink-muted">{m.bots.tokenHint}</p>
      <CopyField
        label={format(m.bots.tokenHeading, { name })}
        value={token}
        copyLabel={m.bots.copyToken}
      />
    </section>
  );
}

/** Text to copy, shown read-only beside the button that copies it. */
function CopyField({
  label,
  value,
  copyLabel,
}: {
  label: string;
  value: string;
  copyLabel: string;
}) {
  const m = useMessages();
  const button = useRef<HTMLButtonElement>(null);
  const [copied, setCopied] = useState(false);
  return (
    <div className="flex items-center gap-2">
      <input
        aria-label={label}
        readOnly
        value={value}
        onFocus={(event) => {
          event.currentTarget.select();
        }}
        className={inputClass + " min-w-0 flex-1 font-mono text-xs"}
      />
      <Button
        ref={button}
        onPress={() => {
          if (button.current !== null) {
            void copyText(value, button.current).then(setCopied);
          }
        }}
        className={secondaryButtonClass + " flex shrink-0 items-center gap-1.5"}
      >
        <CopyIcon size={14} aria-hidden="true" />
        {copied ? m.bots.copied : copyLabel}
      </Button>
    </div>
  );
}
