import type { Community, CommunityUpdateRequest, User } from "@aspen/protocol";
import { CaretDownIcon, WrenchIcon } from "@phosphor-icons/react";
import { useNavigate } from "@tanstack/react-router";
import { useState } from "react";
import {
  Button,
  Dialog,
  DialogTrigger,
  Form,
  Input,
  Label,
  ListBox,
  ListBoxItem,
  Modal,
  ModalOverlay,
  Popover,
  Select,
  SelectValue,
  Tab,
  TabList,
  TabPanel,
  Tabs,
  Text,
  TextField,
  type Key,
} from "react-aria-components";
import {
  useAccess,
  useChannels,
  useMe,
  useMemberRoles,
  useNickname,
  useSync,
  useUser,
} from "@/api/hooks";
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
import { EmojiPanel } from "@/features/community-settings/EmojiPanel";
import { LayoutPanel } from "@/features/community-settings/LayoutPanel";
import { MembersPanel } from "@/features/community-settings/MembersPanel";
import { RolesPanel } from "@/features/community-settings/RolesPanel";
import { PluginsPanel } from "@/features/plugins/PluginsPanel";
import {
  columnPlanesModalClass,
  dangerButtonClass,
  dangerPlaneClass,
  dialogClass,
  optionClass,
  overlayClass,
  planeClass,
  secondaryButtonClass,
  selectButtonClass,
  selectPopoverClass,
  widePlanesModalClass,
} from "@/features/invites/dialog";
import { ChoiceCheckbox } from "@/features/layout/choices";
import { DialogHeading } from "@/features/layout/DialogHeading";
import { CommunityNotifications } from "@/features/notifications/CommunityNotifications";
import { Tooltip } from "@/features/layout/Tooltip";
import { displayNameOf } from "@/features/users/profile";
import { NicknameForm } from "@/features/users/Nickname";
import { useNameIn } from "@/features/users/nameIn";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";
import { problemText } from "@/api/problemText";

const tabClass =
  "cursor-default rounded-md px-3 py-1.5 text-sm outline-none hover:bg-surface-hover " +
  "selected:bg-accent-soft selected:text-accent-strong focus-visible:ring-2 focus-visible:ring-accent/50";

/**
 * The wrench in a community's sidebar, which opens its settings (`CommunitySettingsModal`), as
 * its name above the channels does too (`isOpen`, `onOpenChange`).
 */
export function CommunitySettingsDialog({
  community,
  triggerClassName,
  isOpen,
  onOpenChange,
}: {
  community: Community;
  triggerClassName: string;
  isOpen?: boolean;
  onOpenChange?: (open: boolean) => void;
}) {
  const m = useMessages();
  return (
    <DialogTrigger
      {...(isOpen === undefined ? {} : { isOpen })}
      {...(onOpenChange === undefined ? {} : { onOpenChange })}
    >
      <Tooltip text={m.communitySettings.open}>
        <Button aria-label={m.communitySettings.open} className={triggerClassName}>
          <WrenchIcon size={18} aria-hidden="true" />
        </Button>
      </Tooltip>
      <CommunitySettingsModal community={community} />
    </DialogTrigger>
  );
}

/**
 * The community's settings: its name, its owner (who alone may hand it on or delete it) and the
 * way out of it, for holders of Manage channels or Manage categories the arrangement of its
 * channels and categories, its roles, its members, its emoji, and, for holders of Manage
 * plugins, the deployment's plugins. Every member can open it; each part offers only what they
 * may do. Opened by the trigger it is in, or by `isOpen` (from the community's menu on the
 * rail). It is as wide as Settings while the Roles tab is open, whose permissions stand in up to
 * four columns (`PermissionChecklist`), and narrower for the rest, which read best as a column.
 */
export function CommunitySettingsModal({
  community,
  isOpen,
  onOpenChange,
}: {
  community: Community;
  isOpen?: boolean;
  onOpenChange?: (open: boolean) => void;
}) {
  return (
    <ModalOverlay
      className={overlayClass}
      isDismissable
      {...(isOpen === undefined ? {} : { isOpen })}
      {...(onOpenChange === undefined ? {} : { onOpenChange })}
    >
      <CommunitySettingsFrame community={community} />
    </ModalOverlay>
  );
}

/** The settings' modal, drawn while it is open, so each opening starts on the first tab. */
function CommunitySettingsFrame({ community }: { community: Community }) {
  const m = useMessages();
  const access = useAccess(community.id);
  const manageRoles = access?.has("manageRoles") ?? false;
  const roleTab = manageRoles || (access?.has("assignRoles") ?? false);
  const layoutTab =
    (access?.has("manageChannels") ?? false) || (access?.has("manageCategories") ?? false);
  const pluginTab = access?.has("managePlugins") === true;
  const [tab, setTab] = useState<Key>("overview");
  return (
    <Modal className={tab === "roles" ? columnPlanesModalClass : widePlanesModalClass}>
      <Dialog className={dialogClass}>
        <DialogHeading>
          {format(m.communitySettings.heading, { community: community.name })}
        </DialogHeading>
        <Tabs selectedKey={tab} onSelectionChange={setTab} className="flex flex-col gap-4">
          <TabList aria-label={m.communitySettings.open} className="flex flex-wrap gap-1">
            <Tab id="overview" className={tabClass}>
              {m.communitySettings.overviewTab}
            </Tab>
            {layoutTab && (
              <Tab id="layout" className={tabClass}>
                {m.channelLayout.tab}
              </Tab>
            )}
            {roleTab && (
              <Tab id="roles" className={tabClass}>
                {m.communitySettings.rolesTab}
              </Tab>
            )}
            <Tab id="members" className={tabClass}>
              {m.communitySettings.membersTab}
            </Tab>
            <Tab id="emoji" className={tabClass}>
              {m.communitySettings.emojiTab}
            </Tab>
            {pluginTab && (
              <Tab id="plugins" className={tabClass}>
                {m.plugins.tab}
              </Tab>
            )}
          </TabList>
          <TabPanel id="overview" className="outline-none">
            <Overview community={community} />
          </TabPanel>
          {layoutTab && (
            <TabPanel id="layout" className="outline-none">
              <LayoutPanel communityId={community.id} />
            </TabPanel>
          )}
          {roleTab && (
            <TabPanel id="roles" className="outline-none">
              <RolesPanel communityId={community.id} />
            </TabPanel>
          )}
          <TabPanel id="members" className="outline-none">
            <MembersPanel communityId={community.id} />
          </TabPanel>
          <TabPanel id="emoji" className="outline-none">
            <EmojiPanel communityId={community.id} />
          </TabPanel>
          {pluginTab && (
            <TabPanel id="plugins" className="outline-none">
              <PluginsPanel communityId={community.id} />
            </TabPanel>
          )}
        </Tabs>
      </Dialog>
    </Modal>
  );
}

function Overview({ community }: { community: Community }) {
  const m = useMessages();
  const access = useAccess(community.id);
  const owner = useUser(community.owner ?? undefined);
  const ownerName = useNameIn(owner, community.id);
  const me = useMe();
  const member = useMemberRoles(community.id, me?.id ?? "") !== undefined;
  const manage = access?.has("manageCommunity") === true;
  // A moderator of the server may rename and delete any community, and nothing else here.
  const moderator = access?.moderator === true;
  return (
    <div className="flex flex-col gap-4">
      {moderator && !member && <p className={hintClass}>{m.communitySettings.moderatorNote}</p>}
      {(manage || moderator) && <Rename community={community} icon={manage} />}
      {/* A deployment that predates default channels leaves the field out. */}
      {manage && community.defaultChannel !== undefined && <Joining community={community} />}
      {member && (
        <div className={planeClass}>
          <CommunityNotifications communityId={community.id} />
        </div>
      )}
      {member && me !== null && <OwnNickname communityId={community.id} me={me} />}
      <section className={planeClass}>
        <h3 className="text-sm font-semibold text-ink-muted">{m.communitySettings.ownerHeading}</h3>
        <p className={hintClass}>
          {owner === undefined
            ? m.communitySettings.noOwner
            : format(m.communitySettings.ownerIs, { name: ownerName ?? displayNameOf(owner) })}
        </p>
        {access?.owner === true && <Transfer community={community} />}
      </section>
      {access?.owner === true || moderator ? <Delete community={community} /> : null}
      {access?.owner !== true && member && <Leave community={community} />}
    </div>
  );
}

/** The caller's nickname here, on a plane of its own when there is anything to show. */
function OwnNickname({ communityId, me }: { communityId: string; me: User }) {
  const access = useAccess(communityId);
  const nickname = useNickname(communityId, me.id);
  if (access?.has("changeNickname") !== true && nickname === undefined) {
    return null;
  }
  return (
    <div className={planeClass}>
      <NicknameForm communityId={communityId} me={me} />
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

/**
 * Where new members start and are announced (the community's default channel, one of the text
 * channels the reader can see), and whether they are announced at all. Each change is saved as
 * it is made.
 */
function Joining({ community }: { community: Community }) {
  const m = useMessages();
  const sync = useSync();
  const textChannels = useChannels(community.id).filter((c) => c.ty === "text");
  const [error, setError] = useState<string | null>(null);
  const save = (patch: CommunityUpdateRequest) => {
    setError(null);
    sync.updateCommunity(community.id, patch).catch((e: unknown) => {
      setError(problemText(e));
    });
  };
  return (
    <section className={planeClass}>
      <h3 className="text-sm font-semibold text-ink-muted">{m.communitySettings.joiningHeading}</h3>
      {textChannels.length === 0 ? (
        <p className={hintClass}>{m.communitySettings.noTextChannels}</p>
      ) : (
        <Select
          value={community.defaultChannel ?? null}
          placeholder={
            community.defaultChannel == null
              ? m.communitySettings.noDefaultChannel
              : m.communitySettings.defaultChannelHidden
          }
          onChange={(key) => {
            if (key !== null && key !== community.defaultChannel) {
              save({ defaultChannel: String(key) });
            }
          }}
          className={fieldClass + " max-w-sm"}
        >
          <Label className={labelClass}>{m.communitySettings.defaultChannelLabel}</Label>
          <Button className={selectButtonClass + " py-2"}>
            <SelectValue />
            <CaretDownIcon size={14} aria-hidden="true" className="text-ink-muted" />
          </Button>
          <Text slot="description" className={hintClass}>
            {m.communitySettings.defaultChannelHint}
          </Text>
          <Popover className={selectPopoverClass}>
            <ListBox>
              {textChannels.map((c) => (
                <ListBoxItem key={c.id} id={c.id} textValue={c.name} className={optionClass}>
                  # {c.name}
                </ListBoxItem>
              ))}
            </ListBox>
          </Popover>
        </Select>
      )}
      <ChoiceCheckbox
        isSelected={community.suppressJoinMessages ?? false}
        onChange={(suppressJoinMessages) => {
          save({ suppressJoinMessages });
        }}
        label={m.communitySettings.suppressJoinMessages}
        hint={m.communitySettings.suppressJoinMessagesHint}
      />
      {error !== null && (
        <p role="alert" className={alertClass}>
          {error}
        </p>
      )}
    </section>
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
    <section className={dangerPlaneClass}>
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
