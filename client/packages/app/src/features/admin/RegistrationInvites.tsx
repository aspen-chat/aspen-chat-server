import { ApiProblemError, type RegistrationInvite } from "@aspen/protocol";
import {
  CaretDownIcon,
  CheckCircleIcon,
  ClockCountdownIcon,
  CopyIcon,
  ProhibitIcon,
  QrCodeIcon,
  UserCircleCheckIcon,
} from "@phosphor-icons/react";
import { useCallback, useRef, useState } from "react";
import {
  Button,
  Dialog,
  Form,
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
  TextField,
} from "react-aria-components";
import { useCommunitiesWhere, useSync } from "@/api/hooks";
import { ReadFailed, Section } from "@/features/admin/AdminDashboard";
import { Cell, Status, Table } from "@/features/admin/FleetHealth";
import { useFigures } from "@/features/admin/format";
import { useAdminRead } from "@/features/admin/useAdminRead";
import {
  alertClass,
  fieldClass,
  inputClass,
  labelClass,
  primaryButtonClass,
} from "@/features/auth/styles";
import {
  dangerButtonClass,
  dialogClass,
  modalClass,
  optionClass,
  overlayClass,
  selectButtonClass,
  selectPopoverClass,
} from "@/features/invites/dialog";
import { registrationPath } from "@/features/invites/inviteCode";
import { copyText } from "@/features/layout/clipboard";
import { InviteQr } from "@/features/qr/InviteQr";
import { useShareUrl } from "@/features/qr/shareLinks";
import { DialogHeading } from "@/features/layout/DialogHeading";
import { useMessages } from "@/i18n/context";
import { format, type Messages } from "@/i18n/messages";
import { CopyIdButton } from "@/features/layout/CopyId";

const EXPIRIES: readonly { key: keyof Messages["admin"]["expiry"]; seconds: number | null }[] = [
  { key: "never", seconds: null },
  { key: "day", seconds: 86_400 },
  { key: "week", seconds: 7 * 86_400 },
  { key: "month", seconds: 30 * 86_400 },
];

/** The most accounts one invite may create, as the server's `MAX_USES`. */
const MAX_USES = 1000;
const MAX_NOTE_CHARS = 200;

const smallButtonClass =
  "tap-target inline-flex items-center gap-1 rounded-md px-2 py-1 text-xs text-ink-muted outline-none " +
  "hover:bg-surface-hover hover:text-ink pressed:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50";

/**
 * Making, listing, and revoking the invites that create accounts, each with its link and QR code;
 * one made here shows its QR code at once. A dual invite also joins a community, named beside it.
 */
export function RegistrationInvites({
  inviteRequired,
}: {
  /** Whether the server requires an invite to register; `undefined` until known. */
  inviteRequired: boolean | undefined;
}) {
  const m = useMessages();
  const { count, day, moment } = useFigures();
  const sync = useSync();
  const load = useCallback(() => sync.admin.registrationInvites(), [sync]);
  const invites = useAdminRead(load);
  const [revoking, setRevoking] = useState<string | null>(null);
  const [showing, setShowing] = useState<string | null>(null);
  const now = invites.at;
  return (
    <Section
      id="admin-invites"
      title={m.admin.invites}
      hint={inviteRequired === false ? m.admin.invitesHintOpen : m.admin.invitesHint}
    >
      <CreateInvite
        onCreated={(code) => {
          invites.reload();
          setShowing(code);
        }}
      />
      {invites.error !== null && <ReadFailed error={invites.error} onRetry={invites.reload} />}
      {invites.data?.length === 0 ? (
        <p className="text-sm text-ink-muted">{m.admin.noInvites}</p>
      ) : (
        <Table
          label={m.admin.invites}
          headings={[
            m.admin.code,
            m.admin.status,
            m.admin.uses,
            m.admin.note,
            m.admin.created,
            m.admin.expires,
            m.admin.actions,
          ]}
          numeric={[2]}
          skeletonRows={invites.data === undefined && invites.error === null ? 3 : 0}
        >
          {(invites.data ?? []).map((invite) => (
            <tr key={invite.code}>
              <Cell>
                <code className="font-mono">{invite.code}</code>
              </Cell>
              <Cell>
                <InviteStatus invite={invite} now={now} />
              </Cell>
              <Cell numeric>
                {format(m.admin.used, { uses: count(invite.uses), max: count(invite.maxUses) })}
              </Cell>
              <Cell>
                <span className="line-clamp-2 break-words">{invite.note ?? ""}</span>
                {invite.community != null && (
                  <CommunityJoined invite={invite} community={invite.community} />
                )}
              </Cell>
              <Cell>{day(invite.createdAt)}</Cell>
              <Cell>
                {invite.expiresAt == null ? m.admin.expiry.never : moment(invite.expiresAt)}
              </Cell>
              <Cell>
                <span className="flex justify-end gap-1">
                  {invite.usable && <CopyInvite code={invite.code} />}
                  {invite.usable && (
                    <Button
                      aria-label={format(m.qr.inviteLabel, { code: invite.code })}
                      onPress={() => {
                        setShowing(invite.code);
                      }}
                      className={smallButtonClass}
                    >
                      <QrCodeIcon size={14} aria-hidden="true" />
                      {m.qr.qrCode}
                    </Button>
                  )}
                  {invite.usable && (
                    <Button
                      aria-label={format(m.admin.revokeLabel, { code: invite.code })}
                      onPress={() => {
                        setRevoking(invite.code);
                      }}
                      className={smallButtonClass + " text-danger hover:text-danger"}
                    >
                      {m.admin.revoke}
                    </Button>
                  )}
                  <CopyIdButton id={invite.code} thing="registrationInvite" />
                </span>
              </Cell>
            </tr>
          ))}
        </Table>
      )}
      <QrDialog
        code={showing}
        onClose={() => {
          setShowing(null);
        }}
      />
      <RevokeDialog
        code={revoking}
        onClose={() => {
          setRevoking(null);
        }}
        onRevoked={invites.reload}
      />
    </Section>
  );
}

/**
 * The community a dual invite joins. While the dual invite still makes accounts, a community
 * invite that no longer works is called out, since those accounts would join nothing.
 */
function CommunityJoined({
  invite,
  community,
}: {
  invite: RegistrationInvite;
  community: NonNullable<RegistrationInvite["community"]>;
}) {
  const m = useMessages();
  const gone = invite.usable && !community.usable;
  return (
    <span className={"block text-xs " + (gone ? "text-danger" : "text-ink-muted")}>
      {format(gone ? m.qr.dualJoinsGone : m.qr.dualJoins, { community: community.name })}
    </span>
  );
}

function InviteStatus({ invite, now }: { invite: RegistrationInvite; now: number }) {
  const m = useMessages();
  if (invite.revokedAt != null) {
    return <Status icon={ProhibitIcon} tone="text-danger" label={m.admin.revoked} />;
  }
  if (invite.uses >= invite.maxUses) {
    return <Status icon={UserCircleCheckIcon} tone="text-ink-muted" label={m.admin.usedUp} />;
  }
  if (invite.expiresAt != null && Date.parse(invite.expiresAt) <= now) {
    return <Status icon={ClockCountdownIcon} tone="text-away" label={m.admin.expired} />;
  }
  return <Status icon={CheckCircleIcon} tone="text-online" label={m.admin.usable} />;
}

/** Copies an invite's link (`shareUrl`), once the deployment has said where links point. */
function CopyInvite({ code }: { code: string }) {
  const m = useMessages();
  const share = useShareUrl();
  const [copied, setCopied] = useState(false);
  const button = useRef<HTMLButtonElement>(null);
  return (
    <Button
      ref={button}
      isDisabled={share === null}
      onPress={() => {
        if (button.current !== null && share !== null) {
          void copyText(share(registrationPath(code)), button.current).then(setCopied);
        }
      }}
      className={smallButtonClass}
    >
      <CopyIcon size={14} aria-hidden="true" />
      {copied ? m.admin.copied : m.admin.copyLink}
    </Button>
  );
}

/** An invite's link and QR code, opened from its row or on its making. */
function QrDialog({ code, onClose }: { code: string | null; onClose: () => void }) {
  const m = useMessages();
  const share = useShareUrl();
  const link = code === null || share === null ? null : share(registrationPath(code));
  return (
    <ModalOverlay
      isOpen={code !== null}
      onOpenChange={(open) => {
        if (!open) {
          onClose();
        }
      }}
      isDismissable
      className={overlayClass}
    >
      <Modal className={modalClass}>
        <Dialog className={dialogClass}>
          <DialogHeading>{format(m.qr.inviteLabel, { code: code ?? "" })}</DialogHeading>
          {link !== null && code !== null && (
            <>
              <code className="break-all rounded bg-surface px-2 py-1 font-mono text-xs select-all">
                {link}
              </code>
              <InviteQr
                link={link}
                label={format(m.qr.inviteLabel, { code })}
                fileName={`aspen-registration-${code}`}
              />
            </>
          )}
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}

/** The `community` choice that makes a plain registration invite. */
const NO_COMMUNITY = "none";

function CreateInvite({ onCreated }: { onCreated: (code: string) => void }) {
  const m = useMessages();
  const sync = useSync();
  // A dual invite's community invite is the caller's own, so only communities where they may
  // make invites are offered.
  const communities = useCommunitiesWhere("createInvites");
  const [uses, setUses] = useState(1);
  const [expiry, setExpiry] = useState<string>("week");
  const [note, setNote] = useState("");
  const [community, setCommunity] = useState<string>(NO_COMMUNITY);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function create() {
    setPending(true);
    setError(null);
    try {
      const seconds = EXPIRIES.find((e) => e.key === expiry)?.seconds ?? null;
      const made = await sync.admin.createRegistrationInvite({
        maxUses: uses,
        ...(seconds === null ? {} : { expiresInSeconds: seconds }),
        ...(note.trim() === "" ? {} : { note: note.trim() }),
        ...(community === NO_COMMUNITY ? {} : { community }),
      });
      setNote("");
      onCreated(made.code);
    } catch (e) {
      setError(e instanceof ApiProblemError ? e.message : String(e));
    }
    setPending(false);
  }

  return (
    <Form
      aria-label={m.admin.newInvite}
      onSubmit={(event) => {
        event.preventDefault();
        void create();
      }}
      className="flex flex-col gap-3 rounded-lg border border-line bg-surface-raised p-4"
    >
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-[8rem_12rem_1fr]">
        <NumberField
          value={uses}
          onChange={(value) => {
            setUses(Number.isNaN(value) ? 1 : value);
          }}
          minValue={1}
          maxValue={MAX_USES}
          className={fieldClass}
        >
          <Label className={labelClass}>{m.admin.uses}</Label>
          <Input className={inputClass} />
        </NumberField>
        <Select
          value={expiry}
          onChange={(key) => {
            setExpiry(String(key));
          }}
          className={fieldClass}
        >
          <Label className={labelClass}>{m.admin.expires}</Label>
          <Button className={selectButtonClass + " py-2"}>
            <SelectValue />
            <CaretDownIcon size={14} aria-hidden="true" className="text-ink-muted" />
          </Button>
          <Popover className={selectPopoverClass}>
            <ListBox>
              {EXPIRIES.map((e) => (
                <ListBoxItem key={e.key} id={e.key} className={optionClass}>
                  {m.admin.expiry[e.key]}
                </ListBoxItem>
              ))}
            </ListBox>
          </Popover>
        </Select>
        <TextField
          value={note}
          onChange={setNote}
          maxLength={MAX_NOTE_CHARS}
          className={fieldClass}
        >
          <Label className={labelClass}>{m.admin.note}</Label>
          <Input placeholder={m.admin.notePlaceholder} className={inputClass} />
        </TextField>
      </div>
      {communities.length > 0 && (
        <Select
          value={community}
          onChange={(key) => {
            setCommunity(String(key));
          }}
          className={fieldClass + " sm:max-w-sm"}
        >
          <Label className={labelClass}>{m.qr.dualCommunity}</Label>
          <Button className={selectButtonClass + " py-2"}>
            <SelectValue />
            <CaretDownIcon size={14} aria-hidden="true" className="text-ink-muted" />
          </Button>
          <Popover className={selectPopoverClass}>
            <ListBox>
              <ListBoxItem id={NO_COMMUNITY} className={optionClass}>
                {m.qr.dualNoCommunity}
              </ListBoxItem>
              {communities.map((c) => (
                <ListBoxItem key={c.id} id={c.id} textValue={c.name} className={optionClass}>
                  {c.name}
                </ListBoxItem>
              ))}
            </ListBox>
          </Popover>
        </Select>
      )}
      {error !== null && (
        <p role="alert" className={alertClass}>
          {error}
        </p>
      )}
      <Button type="submit" isDisabled={pending} className={primaryButtonClass + " self-start"}>
        {pending ? m.admin.creating : m.admin.create}
      </Button>
    </Form>
  );
}

function RevokeDialog({
  code,
  onClose,
  onRevoked,
}: {
  code: string | null;
  onClose: () => void;
  onRevoked: () => void;
}) {
  const m = useMessages();
  const sync = useSync();
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function revoke(target: string) {
    setPending(true);
    setError(null);
    try {
      await sync.admin.revokeRegistrationInvite(target);
      onRevoked();
      onClose();
    } catch (e) {
      setError(e instanceof ApiProblemError ? e.message : String(e));
    }
    setPending(false);
  }

  return (
    <ModalOverlay
      isOpen={code !== null}
      onOpenChange={(open) => {
        if (!open) {
          setError(null);
          onClose();
        }
      }}
      isDismissable
      className={overlayClass}
    >
      <Modal className={modalClass}>
        <Dialog role="alertdialog" className={dialogClass}>
          <DialogHeading>{m.admin.revokeHeading}</DialogHeading>
          <p className="text-sm text-ink-muted">
            {format(m.admin.revokeHint, { code: code ?? "" })}
          </p>
          {error !== null && (
            <p role="alert" className="text-sm text-danger">
              {error}
            </p>
          )}
          <Button
            isDisabled={pending}
            onPress={() => {
              if (code !== null) {
                void revoke(code);
              }
            }}
            className={dangerButtonClass + " self-end"}
          >
            {pending ? m.admin.revoking : m.admin.revoke}
          </Button>
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}
