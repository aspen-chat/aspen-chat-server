import { ApiProblemError, type User } from "@aspen/protocol";
import { PencilSimpleIcon, SmileyIcon, XIcon } from "@phosphor-icons/react";
import { lazy, Suspense, useState } from "react";
import {
  Button,
  Dialog,
  DialogTrigger,
  Form,
  Input,
  Label,
  Modal,
  ModalOverlay,
  Popover,
  TextArea,
  TextField,
} from "react-aria-components";
import { useSync } from "@/api/hooks";
import {
  alertClass,
  fieldClass,
  inputClass,
  labelClass,
  primaryButtonClass,
} from "@/features/auth/styles";
import {
  dialogClass,
  modalClass,
  overlayClass,
  secondaryButtonClass,
} from "@/features/invites/dialog";
import { Avatar } from "@/features/communities/Avatar";
import { Tooltip } from "@/features/layout/Tooltip";
import { IconPicker } from "@/features/media/IconPicker";
import { profileForm, profilePatch, type ProfileForm } from "@/features/users/profile";
import { DialogHeading } from "@/features/layout/DialogHeading";
import { useMessages } from "@/i18n/context";

/** The emoji picker is a sizeable chunk, fetched the first time anyone opens it. */
const EmojiPicker = lazy(() => import("@/features/messages/EmojiPicker"));

/** Bounds mirrored from the server's `app::user`, so the form refuses what it would refuse. */
const DISPLAY_NAME_MAX_CHARS = 32;
const PRONOUNS_MAX_CHARS = 40;
const BIO_MAX_CHARS = 1000;
const STATUS_MAX_CHARS = 200;

const iconButtonClass =
  "rounded-md border border-line p-1.5 text-ink-muted outline-none hover:bg-surface-hover hover:text-ink " +
  "pressed:bg-surface-hover disabled:opacity-40 focus-visible:ring-2 focus-visible:ring-accent/50";

/** The signed-in user's "Edit profile" control and the form it opens. */
export function EditProfileDialog({
  user,
  triggerClassName,
}: {
  user: User;
  triggerClassName: string;
}) {
  const m = useMessages();
  return (
    <DialogTrigger>
      <Tooltip text={m.profile.edit}>
        <Button aria-label={m.profile.edit} className={triggerClassName}>
          <PencilSimpleIcon size={16} aria-hidden="true" />
        </Button>
      </Tooltip>
      <ModalOverlay className={overlayClass} isDismissable>
        <Modal className={modalClass}>
          <Dialog className={dialogClass}>
            {({ close }) => <ProfileEditor user={user} close={close} />}
          </Dialog>
        </Modal>
      </ModalOverlay>
    </DialogTrigger>
  );
}

function ProfileEditor({ user, close }: { user: User; close: () => void }) {
  const m = useMessages();
  const sync = useSync();
  const [form, setForm] = useState<ProfileForm>(() => profileForm(user));
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  function set<K extends keyof ProfileForm>(key: K, value: ProfileForm[K]) {
    setForm((current) => ({ ...current, [key]: value }));
  }

  async function save() {
    const patch = profilePatch(user, form);
    if (Object.keys(patch).length === 0) {
      close();
      return;
    }
    setPending(true);
    setError(null);
    try {
      await sync.updateProfile(patch);
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
        void save();
      }}
      className="flex flex-col gap-4"
    >
      <DialogHeading>{m.profile.heading}</DialogHeading>
      {error !== null && (
        <p role="alert" className={alertClass}>
          {error}
        </p>
      )}
      <div className={fieldClass}>
        <span className={labelClass}>{m.profile.avatar}</span>
        <div className="flex items-center gap-3">
          <Avatar name={form.displayName.trim() || user.name} iconId={form.icon} size="lg" />
          <IconPicker
            onIcon={(iconId) => {
              set("icon", iconId);
            }}
          >
            {(open, uploading) => (
              <Button onPress={open} isDisabled={uploading} className={secondaryButtonClass}>
                {m.profile.changeAvatar}
              </Button>
            )}
          </IconPicker>
          {form.icon !== null && (
            <Button
              onPress={() => {
                set("icon", null);
              }}
              className={secondaryButtonClass}
            >
              {m.profile.removeAvatar}
            </Button>
          )}
        </div>
      </div>
      <TextField
        value={form.displayName}
        onChange={(value) => {
          set("displayName", value);
        }}
        maxLength={DISPLAY_NAME_MAX_CHARS}
        autoFocus
        className={fieldClass}
      >
        <Label className={labelClass}>{m.profile.displayName}</Label>
        <Input placeholder={user.name} className={inputClass} />
      </TextField>
      <TextField
        value={form.pronouns}
        onChange={(value) => {
          set("pronouns", value);
        }}
        maxLength={PRONOUNS_MAX_CHARS}
        className={fieldClass}
      >
        <Label className={labelClass}>{m.profile.pronouns}</Label>
        <Input placeholder={m.profile.pronounsPlaceholder} className={inputClass} />
      </TextField>
      <div className={fieldClass}>
        <span id="profile-status-label" className={labelClass}>
          {m.profile.status}
        </span>
        <div className="flex items-center gap-1">
          <StatusEmojiPicker
            emoji={form.statusEmoji}
            onChange={(emoji) => {
              set("statusEmoji", emoji);
            }}
          />
          <TextField
            value={form.statusText}
            onChange={(value) => {
              set("statusText", value);
            }}
            maxLength={STATUS_MAX_CHARS}
            aria-labelledby="profile-status-label"
            className="flex-1"
          >
            <Input placeholder={m.profile.statusPlaceholder} className={inputClass + " w-full"} />
          </TextField>
          {form.statusText.length > 0 && (
            <Button
              aria-label={m.profile.clearStatus}
              onPress={() => {
                set("statusText", "");
                set("statusEmoji", null);
              }}
              className={iconButtonClass}
            >
              <XIcon size={16} aria-hidden="true" />
            </Button>
          )}
        </div>
      </div>
      <TextField
        value={form.bio}
        onChange={(value) => {
          set("bio", value);
        }}
        maxLength={BIO_MAX_CHARS}
        className={fieldClass}
      >
        <Label className={labelClass}>{m.profile.bio}</Label>
        <TextArea
          rows={3}
          placeholder={m.profile.bioPlaceholder}
          className={inputClass + " max-h-48 resize-none field-sizing-content"}
        />
      </TextField>
      <div className="flex justify-end gap-2">
        <Button type="submit" isDisabled={pending} className={primaryButtonClass}>
          {pending ? m.profile.saving : m.save}
        </Button>
      </div>
    </Form>
  );
}

/** The status's emoji control: the chosen emoji, or a smiley when there is none. */
function StatusEmojiPicker({
  emoji,
  onChange,
}: {
  emoji: string | null;
  onChange: (emoji: string | null) => void;
}) {
  const m = useMessages();
  const label = emoji === null ? m.profile.pickStatusEmoji : m.profile.changeStatusEmoji;
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
                  communityId={null}
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
