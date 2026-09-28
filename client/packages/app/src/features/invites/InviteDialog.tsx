import { ApiProblemError, type Community, type Invite } from "@aspen/protocol";
import { CaretDownIcon, UserPlusIcon } from "@phosphor-icons/react";
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
} from "react-aria-components";
import { useInvites, useSync } from "@/api/hooks";
import { primaryButtonClass } from "@/features/auth/styles";
import { Tooltip } from "@/features/layout/Tooltip";
import {
  dialogClass,
  modalClass,
  overlayClass,
  secondaryButtonClass,
} from "@/features/invites/dialog";
import { inviteLink } from "@/features/invites/inviteCode";
import { copyText } from "@/features/layout/clipboard";
import { DialogHeading } from "@/features/layout/DialogHeading";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/** How long a new invite stays valid, as a key into the `expiry` messages. */
const EXPIRY_OPTIONS = {
  never: null,
  hour: 60 * 60 * 1000,
  day: 24 * 60 * 60 * 1000,
  week: 7 * 24 * 60 * 60 * 1000,
} as const;
type ExpiryOption = keyof typeof EXPIRY_OPTIONS;

const dateFormat = new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "short" });

const inviteButtonClass =
  "rounded-md border border-line p-1.5 text-ink-muted outline-none hover:bg-surface-hover hover:text-ink " +
  "pressed:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50";

/** The invite control in a community's sidebar and the dialog that manages its invites. */
export function InviteDialog({ community }: { community: Community }) {
  const m = useMessages();
  return (
    <DialogTrigger>
      <Tooltip text={m.invitePeople}>
        <Button aria-label={m.invitePeople} className={inviteButtonClass}>
          <UserPlusIcon size={18} aria-hidden="true" />
        </Button>
      </Tooltip>
      <ModalOverlay className={overlayClass} isDismissable>
        <Modal className={modalClass}>
          <Dialog className={dialogClass}>
            <DialogHeading>
              {format(m.inviteDialogHeading, { community: community.name })}
            </DialogHeading>
            <InviteManager communityId={community.id} />
          </Dialog>
        </Modal>
      </ModalOverlay>
    </DialogTrigger>
  );
}

/** Lists a community's invites with copy and revoke, and creates new ones. */
export function InviteManager({ communityId }: { communityId: string }) {
  const m = useMessages();
  const sync = useSync();
  const invites = useInvites(communityId);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [expiry, setExpiry] = useState<ExpiryOption>("week");
  const [creating, setCreating] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // "Expired" is judged against the moment the dialog opened; it is a label, not a timer.
  const [openedAt] = useState(() => Date.now());

  useEffect(() => {
    sync.loadInvites(communityId).catch((e: unknown) => {
      setLoadError(e instanceof ApiProblemError ? e.message : String(e));
    });
  }, [sync, communityId]);

  async function create() {
    setCreating(true);
    setError(null);
    try {
      const lifetime = EXPIRY_OPTIONS[expiry];
      await sync.createInvite(communityId, {
        expiresAt: lifetime === null ? null : new Date(Date.now() + lifetime).toISOString(),
      });
    } catch (e) {
      setError(e instanceof ApiProblemError ? e.message : String(e));
    } finally {
      setCreating(false);
    }
  }

  return (
    <div className="flex flex-col gap-4">
      <form
        onSubmit={(event) => {
          event.preventDefault();
          void create();
        }}
        className="flex items-end gap-2"
      >
        <Select
          value={expiry}
          onChange={(key) => {
            if (typeof key === "string" && key in EXPIRY_OPTIONS) {
              setExpiry(key as ExpiryOption);
            }
          }}
          className="flex flex-1 flex-col gap-1"
        >
          <Label className="text-sm font-medium text-ink-muted">{m.expiryLabel}</Label>
          <Button className="flex justify-between rounded-md border border-line bg-surface px-3 py-2 text-left outline-none hover:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50">
            <SelectValue />
            <CaretDownIcon size={14} aria-hidden="true" />
          </Button>
          <Popover className="min-w-(--trigger-width) rounded-md border border-line bg-surface-raised p-1 shadow-lg">
            <ListBox className="outline-none">
              {(Object.keys(EXPIRY_OPTIONS) as ExpiryOption[]).map((option) => (
                <ListBoxItem
                  key={option}
                  id={option}
                  textValue={m.expiry[option]}
                  className="cursor-default rounded px-2 py-1 text-sm outline-none focus:bg-surface-hover selected:font-medium selected:text-accent"
                >
                  {m.expiry[option]}
                </ListBoxItem>
              ))}
            </ListBox>
          </Popover>
        </Select>
        <Button type="submit" isDisabled={creating} className={primaryButtonClass}>
          {creating ? m.creatingInvite : m.createInvite}
        </Button>
      </form>
      {error !== null && (
        <p role="alert" className="rounded-md bg-danger-soft px-3 py-2 text-sm text-danger">
          {error}
        </p>
      )}
      {loadError !== null ? (
        <p role="alert" className="text-sm text-danger">
          {loadError}
        </p>
      ) : invites.length === 0 ? (
        <p className="text-sm text-ink-muted">{m.noInvitesYet}</p>
      ) : (
        <ul className="flex max-h-72 flex-col gap-2 overflow-y-auto">
          {invites.map((invite) => (
            <InviteRow key={invite.code} invite={invite} now={openedAt} />
          ))}
        </ul>
      )}
    </div>
  );
}

function InviteRow({ invite, now }: { invite: Invite; now: number }) {
  const m = useMessages();
  const sync = useSync();
  const [copied, setCopied] = useState(false);
  const [copyFailed, setCopyFailed] = useState(false);
  const [revoking, setRevoking] = useState(false);
  const link = inviteLink(invite.code);
  const expiresAt = invite.expiresAt == null ? null : new Date(invite.expiresAt);
  const expired = expiresAt !== null && expiresAt.getTime() < now;

  async function copy(near: Element) {
    if (await copyText(link, near)) {
      setCopyFailed(false);
      setCopied(true);
      setTimeout(() => {
        setCopied(false);
      }, 2000);
    } else {
      setCopyFailed(true);
    }
  }

  // The code is what tells invites apart; the rest of the link is the same for all of them, so
  // it shows only where it is copied from, or when copying failed and it must be copied by hand.
  return (
    <li className="flex flex-col gap-1 rounded-md border border-line px-3 py-2 text-sm">
      <code className="truncate font-mono text-sm" title={link}>
        {invite.code}
      </code>
      {copyFailed && (
        <div className="flex flex-col gap-1">
          <p role="alert" className="text-xs text-ink-muted">
            {m.copyFailed}
          </p>
          <input
            readOnly
            value={link}
            aria-label={m.copyLink}
            ref={(field) => {
              field?.select();
            }}
            onFocus={(event) => {
              event.currentTarget.select();
            }}
            className="w-full rounded border border-line bg-surface px-2 py-1 font-mono text-base md:text-xs"
          />
        </div>
      )}
      <div className="flex items-center gap-2">
        <span className={"flex-1 " + (expired ? "text-danger" : "text-ink-muted")}>
          {expiresAt === null
            ? m.neverExpires
            : expired
              ? m.expired
              : format(m.expiresOn, { date: dateFormat.format(expiresAt) })}
        </span>
        <Button
          onPress={(event) => {
            void copy(event.target);
          }}
          className={secondaryButtonClass}
        >
          {copied ? m.copied : m.copyLink}
        </Button>
        <Button
          isDisabled={revoking}
          onPress={() => {
            setRevoking(true);
            sync.revokeInvite(invite.code).catch(() => {
              setRevoking(false);
            });
          }}
          className={secondaryButtonClass + " text-danger"}
        >
          {m.revoke}
        </Button>
      </div>
    </li>
  );
}
