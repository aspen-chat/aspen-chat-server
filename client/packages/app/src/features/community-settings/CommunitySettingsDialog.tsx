import { ApiProblemError, type Community, type User } from "@aspen/protocol";
import { GearSixIcon } from "@phosphor-icons/react";
import { useNavigate } from "@tanstack/react-router";
import { useState } from "react";
import {
  Button,
  Dialog,
  DialogTrigger,
  Form,
  Input,
  Label,
  Modal,
  ModalOverlay,
  Tab,
  TabList,
  TabPanel,
  Tabs,
  TextField,
} from "react-aria-components";
import { useAccess, useMe, useMemberRoles, useSync, useUser } from "@/api/hooks";
import {
  alertClass,
  fieldClass,
  hintClass,
  inputClass,
  labelClass,
  primaryButtonClass,
} from "@/features/auth/styles";
import { Avatar } from "@/features/communities/Avatar";
import { IconPicker } from "@/features/media/IconPicker";
import { MemberPicker } from "@/features/community-settings/MemberPicker";
import { MembersPanel } from "@/features/community-settings/MembersPanel";
import { RolesPanel } from "@/features/community-settings/RolesPanel";
import {
  dangerButtonClass,
  dialogClass,
  overlayClass,
  planeClass,
  secondaryButtonClass,
  widePlanesModalClass,
} from "@/features/invites/dialog";
import { DialogHeading } from "@/features/layout/DialogHeading";
import { CommunityNotifications } from "@/features/notifications/CommunityNotifications";
import { Tooltip } from "@/features/layout/Tooltip";
import { displayNameOf } from "@/features/users/profile";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

const tabClass =
  "cursor-default rounded-md px-3 py-1.5 text-sm outline-none hover:bg-surface-hover " +
  "selected:bg-accent-soft selected:text-accent-strong focus-visible:ring-2 focus-visible:ring-accent/50";

function problemText(e: unknown): string {
  return e instanceof ApiProblemError ? e.message : String(e);
}

/**
 * The community's settings, from the gear in its sidebar: its name, its owner (who alone may
 * hand it on or delete it) and the way out of it, its roles, and its members. Every member can
 * open it; each part offers only what they may do.
 */
export function CommunitySettingsDialog({
  community,
  triggerClassName,
}: {
  community: Community;
  triggerClassName: string;
}) {
  const m = useMessages();
  const access = useAccess(community.id);
  const manageRoles = access?.has("manageRoles") ?? false;
  const roleTab = manageRoles || (access?.has("assignRoles") ?? false);
  return (
    <DialogTrigger>
      <Tooltip text={m.communitySettings.open}>
        <Button aria-label={m.communitySettings.open} className={triggerClassName}>
          <GearSixIcon size={18} aria-hidden="true" />
        </Button>
      </Tooltip>
      <ModalOverlay className={overlayClass} isDismissable>
        <Modal className={widePlanesModalClass}>
          <Dialog className={dialogClass}>
            <DialogHeading>
              {format(m.communitySettings.heading, { community: community.name })}
            </DialogHeading>
            <Tabs className="flex flex-col gap-4">
              <TabList aria-label={m.communitySettings.open} className="flex gap-1">
                <Tab id="overview" className={tabClass}>
                  {m.communitySettings.overviewTab}
                </Tab>
                {roleTab && (
                  <Tab id="roles" className={tabClass}>
                    {m.communitySettings.rolesTab}
                  </Tab>
                )}
                <Tab id="members" className={tabClass}>
                  {m.communitySettings.membersTab}
                </Tab>
              </TabList>
              <TabPanel id="overview" className="outline-none">
                <Overview community={community} />
              </TabPanel>
              {roleTab && (
                <TabPanel id="roles" className="outline-none">
                  <RolesPanel communityId={community.id} />
                </TabPanel>
              )}
              <TabPanel id="members" className="outline-none">
                <MembersPanel communityId={community.id} />
              </TabPanel>
            </Tabs>
          </Dialog>
        </Modal>
      </ModalOverlay>
    </DialogTrigger>
  );
}

function Overview({ community }: { community: Community }) {
  const m = useMessages();
  const access = useAccess(community.id);
  const owner = useUser(community.owner ?? undefined);
  const me = useMe();
  const member = useMemberRoles(community.id, me?.id ?? "") !== undefined;
  const manage = access?.has("manageCommunity") === true;
  // A moderator of the server may rename and delete any community, and nothing else here.
  const moderator = access?.moderator === true;
  return (
    <div className="flex flex-col gap-4">
      {moderator && !member && <p className={hintClass}>{m.communitySettings.moderatorNote}</p>}
      {(manage || moderator) && <Rename community={community} icon={manage} />}
      {member && (
        <div className={planeClass}>
          <CommunityNotifications communityId={community.id} />
        </div>
      )}
      <section className={planeClass}>
        <h3 className="text-sm font-semibold text-ink-muted">{m.communitySettings.ownerHeading}</h3>
        <p className={hintClass}>
          {owner === undefined
            ? m.communitySettings.noOwner
            : format(m.communitySettings.ownerIs, { name: displayNameOf(owner) })}
        </p>
        {access?.owner === true && <Transfer community={community} />}
      </section>
      {access?.owner === true || moderator ? <Delete community={community} /> : null}
      {access?.owner !== true && member && <Leave community={community} />}
    </div>
  );
}

/** The community's name, and, with `icon`, its icon. */
function Rename({ community, icon }: { community: Community; icon: boolean }) {
  const m = useMessages();
  const sync = useSync();
  const [name, setName] = useState(community.name);
  const [iconError, setIconError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  return (
    <Form
      onSubmit={(event) => {
        event.preventDefault();
        setSaving(true);
        setError(null);
        sync
          .updateCommunity(community.id, { name: name.trim() })
          .catch((e: unknown) => {
            setError(problemText(e));
          })
          .finally(() => {
            setSaving(false);
          });
      }}
      className={planeClass}
    >
      {icon && (
        <div className="flex items-center gap-3">
          <Avatar name={community.name} iconId={community.icon} />
          <IconPicker
            onIcon={async (iconId) => {
              setIconError(null);
              await sync.updateCommunity(community.id, { icon: iconId }).catch((e: unknown) => {
                setIconError(problemText(e));
              });
            }}
          >
            {(open, uploading) => (
              <Button onPress={open} isDisabled={uploading} className={secondaryButtonClass}>
                {m.changeCommunityIcon}
              </Button>
            )}
          </IconPicker>
        </div>
      )}
      {iconError !== null && (
        <p role="alert" className={alertClass}>
          {iconError}
        </p>
      )}
      <TextField value={name} onChange={setName} isRequired className={fieldClass}>
        <Label className={labelClass}>{m.communitySettings.nameLabel}</Label>
        <Input className={inputClass} />
      </TextField>
      {error !== null && (
        <p role="alert" className={alertClass}>
          {error}
        </p>
      )}
      <Button
        type="submit"
        isDisabled={saving || name.trim() === community.name || name.trim() === ""}
        className={primaryButtonClass + " self-start"}
      >
        {saving ? m.communitySettings.saving : m.communitySettings.save}
      </Button>
    </Form>
  );
}

/** The owner's choice of a member to hand the community to, confirmed before it happens. */
function Transfer({ community }: { community: Community }) {
  const m = useMessages();
  const sync = useSync();
  const me = useMe();
  const [chosen, setChosen] = useState<User | null>(null);
  const [confirming, setConfirming] = useState(false);
  const [error, setError] = useState<string | null>(null);
  return (
    <div className="flex flex-col gap-2">
      <MemberPicker
        communityId={community.id}
        label={m.communitySettings.transferLabel}
        exclude={me === null ? [] : [me.id]}
        onChange={(user) => {
          setChosen(user);
          setConfirming(false);
        }}
      />
      {chosen !== null && confirming && (
        <p className="text-sm">
          {format(m.communitySettings.transferConfirm, {
            community: community.name,
            name: displayNameOf(chosen),
          })}
        </p>
      )}
      {error !== null && (
        <p role="alert" className={alertClass}>
          {error}
        </p>
      )}
      <Button
        isDisabled={chosen === null}
        onPress={() => {
          if (chosen === null) {
            return;
          }
          if (!confirming) {
            setConfirming(true);
            return;
          }
          setError(null);
          sync.transferOwnership(community.id, chosen.id).then(
            () => {
              setConfirming(false);
            },
            (e: unknown) => {
              setError(problemText(e));
            },
          );
        }}
        className={(confirming ? dangerButtonClass : secondaryButtonClass) + " self-start"}
      >
        {m.communitySettings.transfer}
      </Button>
    </div>
  );
}

/** Deleting the community, which its owner may and a moderator of the server, confirmed by
 * typing its name. */
function Delete({ community }: { community: Community }) {
  const m = useMessages();
  const sync = useSync();
  const navigate = useNavigate();
  const [typed, setTyped] = useState("");
  const [deleting, setDeleting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  return (
    <section className="flex flex-col gap-2 rounded-lg border border-danger/40 bg-surface-raised p-1.5 shadow-sm">
      <h3 className="text-sm font-semibold text-danger">{m.communitySettings.deleteHeading}</h3>
      <p className={hintClass}>{m.communitySettings.deleteHint}</p>
      <TextField value={typed} onChange={setTyped} className={fieldClass}>
        <Label className={labelClass}>{m.communitySettings.deleteConfirmLabel}</Label>
        <Input className={inputClass} />
      </TextField>
      {error !== null && (
        <p role="alert" className={alertClass}>
          {error}
        </p>
      )}
      <Button
        isDisabled={deleting || typed.trim() !== community.name}
        onPress={() => {
          setDeleting(true);
          setError(null);
          sync.deleteCommunity(community.id).then(
            () => {
              void navigate({ to: "/" });
            },
            (e: unknown) => {
              setError(problemText(e));
              setDeleting(false);
            },
          );
        }}
        className={dangerButtonClass + " self-start"}
      >
        {deleting ? m.communitySettings.deleting : m.communitySettings.delete}
      </Button>
    </section>
  );
}

/** Leaving the community, for anyone but its owner. */
function Leave({ community }: { community: Community }) {
  const m = useMessages();
  const sync = useSync();
  const navigate = useNavigate();
  const [confirming, setConfirming] = useState(false);
  const [error, setError] = useState<string | null>(null);
  return (
    <section className={planeClass}>
      {confirming && (
        <p className="text-sm">
          {format(m.communitySettings.leaveConfirm, { community: community.name })}
        </p>
      )}
      {error !== null && (
        <p role="alert" className={alertClass}>
          {error}
        </p>
      )}
      <Button
        onPress={() => {
          if (!confirming) {
            setConfirming(true);
            return;
          }
          setError(null);
          sync.leaveCommunity(community.id).then(
            () => {
              void navigate({ to: "/" });
            },
            (e: unknown) => {
              setError(problemText(e));
            },
          );
        }}
        className={
          (confirming ? dangerButtonClass : secondaryButtonClass + " text-danger") + " self-start"
        }
      >
        {m.communitySettings.leave}
      </Button>
    </section>
  );
}
