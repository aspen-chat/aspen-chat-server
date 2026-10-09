import {
  ApiProblemError,
  type Community,
  type Invite,
  type RegistrationInvite,
} from "@aspen/protocol";
import { CaretDownIcon, QrCodeIcon, UserPlusIcon } from "@phosphor-icons/react";
import { useEffect, useState } from "react";
import {
  Button,
  Dialog,
  DialogTrigger,
  Input,
  Label,
  ListBox,
  ListBoxItem,
  Modal,
  ModalOverlay,
  NumberField,
  Popover,
  Select,
  SelectValue,
} from "react-aria-components";
import { useCan, useDeploymentCan, useInvites, useMe, useSync } from "@/api/hooks";
import { useScopeDomain } from "@/api/identity";
import { inputClass, labelClass, primaryButtonClass } from "@/features/auth/styles";
import { headerIconButtonClass } from "@/features/layout/headerButton";
import { ShowMore } from "@/features/layout/ShowMore";
import { Tooltip } from "@/features/layout/Tooltip";
import {
  dialogClass,
  modalClass,
  overlayClass,
  secondaryButtonClass,
  selectPopoverClass,
} from "@/features/invites/dialog";
import { invitePath, registrationPath } from "@/features/invites/inviteCode";
import { ChoiceCheckbox } from "@/features/layout/choices";
import { InviteQr } from "@/features/qr/InviteQr";
import { useShareUrl } from "@/features/qr/shareLinks";
import { copyText } from "@/features/layout/clipboard";
import { DialogHeading } from "@/features/layout/DialogHeading";
import { useMessages } from "@/i18n/context";
import { useDateFormat } from "@/i18n/format";
import { format } from "@/i18n/messages";
import { CopyIdButton } from "@/features/layout/CopyId";

/** How long a new invite stays valid, as a key into the `expiry` messages. */
const EXPIRY_OPTIONS = {
  never: null,
  hour: 60 * 60 * 1000,
  day: 24 * 60 * 60 * 1000,
  week: 7 * 24 * 60 * 60 * 1000,
} as const;
type ExpiryOption = keyof typeof EXPIRY_OPTIONS;

const DATE: Intl.DateTimeFormatOptions = { dateStyle: "medium", timeStyle: "short" };

/** The invite control in a community's sidebar and the dialog that manages its invites. */
export function InviteDialog({ community }: { community: Community }) {
  const m = useMessages();
  return (
    <DialogTrigger>
      <Tooltip text={m.invitePeople}>
        <Button aria-label={m.invitePeople} className={headerIconButtonClass}>
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

/** The most accounts one dual invite may make, as the server's `MAX_USES`. */
const MAX_DUAL_USES = 1000;

/**
 * Lists a community's invites with copy, QR code, and revoke, and creates new ones. Without
 * Manage invites the server lists only the caller's own, and without Create invites nothing new
 * is offered. Someone who may also make registration invites may make a dual invite here: a
 * link that creates an account on the deployment and joins this community with it.
 */
export function InviteManager({ communityId }: { communityId: string }) {
  const m = useMessages();
  const sync = useSync();
  const me = useMe();
  const invites = useInvites(communityId);
  const mayCreate = useCan(communityId, "createInvites");
  const manageAll = useCan(communityId, "manageInvites");
  const mayDual = useDeploymentCan("manageRegistrationInvites");
  const [loadError, setLoadError] = useState<string | null>(null);
  const [expiry, setExpiry] = useState<ExpiryOption>("week");
  const [dual, setDual] = useState(false);
  const [dualUses, setDualUses] = useState(1);
  const [creating, setCreating] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // The invite just made shows its QR code at once; any other shows it when asked.
  const [shown, setShown] = useState<string | null>(null);
  const [made, setMade] = useState<RegistrationInvite | null>(null);
  // "Expired" is judged against the moment the dialog opened; it is a label, not a timer.
  const [openedAt] = useState(() => Date.now());

  // Read again when Manage invites is gained or lost, since it decides whose invites are shown
  // and no event brings those made before it was gained.
  useEffect(() => {
    sync.loadInvites(communityId).catch((e: unknown) => {
      setLoadError(e instanceof ApiProblemError ? e.message : String(e));
    });
  }, [sync, communityId, manageAll]);

  async function create() {
    setCreating(true);
    setError(null);
    try {
      const lifetime = EXPIRY_OPTIONS[expiry];
      if (dual && mayDual) {
        setMade(
          await sync.admin.createRegistrationInvite({
            maxUses: dualUses,
            community: communityId,
            ...(lifetime === null ? {} : { expiresInSeconds: lifetime / 1000 }),
          }),
        );
        setShown(null);
      } else {
        const invite = await sync.createInvite(communityId, {
          expiresAt: lifetime === null ? null : new Date(Date.now() + lifetime).toISOString(),
        });
        setMade(null);
        setShown(invite.code);
      }
    } catch (e) {
      setError(e instanceof ApiProblemError ? e.message : String(e));
    } finally {
      setCreating(false);
    }
  }

  return (
    <div className="flex flex-col gap-4">
      {mayCreate && (
        <form
          onSubmit={(event) => {
            event.preventDefault();
            void create();
          }}
          className="flex flex-col gap-3"
        >
          <div className="flex items-end gap-2">
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
              <Button className="flex justify-between rounded-md border border-line bg-surface px-3 py-2 text-start outline-none hover:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50">
                <SelectValue />
                <CaretDownIcon size={14} aria-hidden="true" />
              </Button>
              <Popover className={selectPopoverClass}>
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
            {dual && mayDual && (
              <NumberField
                value={dualUses}
                onChange={(value) => {
                  setDualUses(Number.isNaN(value) ? 1 : value);
                }}
                minValue={1}
                maxValue={MAX_DUAL_USES}
                className="flex w-24 flex-col gap-1"
              >
                <Label className={labelClass}>{m.qr.dualUses}</Label>
                <Input className={inputClass} />
              </NumberField>
            )}
            <Button type="submit" isDisabled={creating} className={primaryButtonClass}>
              {creating ? m.creatingInvite : m.createInvite}
            </Button>
          </div>
          {mayDual && (
            <ChoiceCheckbox
              isSelected={dual}
              onChange={setDual}
              label={m.qr.dualLabel}
              hint={m.qr.dualHint}
            />
          )}
        </form>
      )}
      {error !== null && (
        <p role="alert" className="rounded-md bg-danger-soft px-3 py-2 text-sm text-danger">
          {error}
        </p>
      )}
      {made !== null && <MadeDualInvite invite={made} />}
      {loadError !== null ? (
        <p role="alert" className="text-sm text-danger">
          {loadError}
        </p>
      ) : invites.length === 0 ? (
        <p className="text-sm text-ink-muted">{m.noInvitesYet}</p>
      ) : (
        <ul className="flex max-h-96 flex-col gap-2 overflow-y-auto">
          {invites.map((invite) => (
            <InviteRow
              key={invite.code}
              invite={invite}
              now={openedAt}
              revocable={manageAll || invite.createdBy === me?.id}
              qrShown={shown === invite.code}
              onToggleQr={() => {
                setShown((current) => (current === invite.code ? null : invite.code));
              }}
            />
          ))}
        </ul>
      )}
      {loadError === null && (
        <ShowMore
          topic={`invites:${communityId}`}
          load={() => sync.loadInvites(communityId, true)}
        />
      )}
    </div>
  );
}

/**
 * A dual invite just made: its registration link, which is what to share, and its QR code. Its
 * community invite is listed below with the others, like any invite of the community.
 */
function MadeDualInvite({ invite }: { invite: RegistrationInvite }) {
  const m = useMessages();
  const share = useShareUrl();
  const [copied, setCopied] = useState(false);
  if (share === null) {
    return null;
  }
  const link = share(registrationPath(invite.code));
  return (
    <section
      aria-label={m.qr.dualMade}
      className="flex flex-col gap-2 rounded-md border border-accent/40 px-3 py-3 text-sm"
    >
      <p className="font-medium">{m.qr.dualMade}</p>
      <p className="text-ink-muted">{m.qr.dualMadeHint}</p>
      <code className="break-all font-mono text-xs">{link}</code>
      <InviteQr
        link={link}
        label={format(m.qr.inviteLabel, { code: invite.code })}
        fileName={`aspen-invite-${invite.code}`}
      />
      <Button
        onPress={(event) => {
          void copyText(link, event.target).then(setCopied);
        }}
        className={secondaryButtonClass + " self-start"}
      >
        {copied ? m.copied : m.copyLink}
      </Button>
    </section>
  );
}

function InviteRow({
  invite,
  now,
  revocable,
  qrShown,
  onToggleQr,
}: {
  invite: Invite;
  now: number;
  revocable: boolean;
  qrShown: boolean;
  onToggleQr: () => void;
}) {
  const dateFormat = useDateFormat(DATE);
  const m = useMessages();
  const sync = useSync();
  const [copied, setCopied] = useState(false);
  const [copyFailed, setCopyFailed] = useState(false);
  const [revoking, setRevoking] = useState(false);
  const domain = useScopeDomain();
  const share = useShareUrl();
  const link = share?.(invitePath(invite.code, domain)) ?? null;
  const expiresAt = invite.expiresAt == null ? null : new Date(invite.expiresAt);
  const expired = expiresAt !== null && expiresAt.getTime() < now;

  async function copy(near: Element) {
    if (link !== null && (await copyText(link, near))) {
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
      <code className="truncate font-mono text-sm" title={link ?? undefined}>
        {invite.code}
      </code>
      {copyFailed && (
        <div className="flex flex-col gap-1">
          <p role="alert" className="text-xs text-ink-muted">
            {m.copyFailed}
          </p>
          <input
            readOnly
            value={link ?? ""}
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
          isDisabled={link === null}
          onPress={(event) => {
            void copy(event.target);
          }}
          className={secondaryButtonClass}
        >
          {copied ? m.copied : m.copyLink}
        </Button>
        <Tooltip text={m.qr.show}>
          <Button
            aria-label={m.qr.show}
            aria-expanded={qrShown}
            isDisabled={link === null}
            onPress={onToggleQr}
            className={secondaryButtonClass + " flex items-center"}
          >
            <QrCodeIcon size={16} aria-hidden="true" />
          </Button>
        </Tooltip>
        {revocable && (
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
        )}
        <CopyIdButton id={invite.code} thing="invite" />
      </div>
      {qrShown && link !== null && (
        <InviteQr
          link={link}
          label={format(m.qr.inviteLabel, { code: invite.code })}
          fileName={`aspen-invite-${invite.code}`}
        />
      )}
    </li>
  );
}
