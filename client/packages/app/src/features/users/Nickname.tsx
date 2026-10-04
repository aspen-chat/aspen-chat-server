import type { User } from "@aspen/protocol";
import { FlagIcon } from "@phosphor-icons/react";
import { useState } from "react";
import { Button, DialogTrigger, Form, Input, Label, TextField } from "react-aria-components";
import { useAccess, useCommunity, useNickname, useSync } from "@/api/hooks";
import { problemText } from "@/api/problemText";
import {
  alertClass,
  fieldClass,
  hintClass,
  inputClass,
  labelClass,
  primaryButtonClass,
} from "@/features/auth/styles";
import { secondaryButtonClass } from "@/features/invites/dialog";
import { toast } from "@/features/layout/toast";
import { ReportModal } from "@/features/reports/ReportDialog";
import { useMayClearNickname } from "@/features/users/nameIn";
import { displayNameOf } from "@/features/users/profile";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/**
 * The caller's own nickname in a community: a field to choose one, with Change nickname, and a
 * way to clear the one they have, which needs nothing. Someone who may not choose one and has
 * none is shown nothing.
 */
export function NicknameForm({ communityId, me }: { communityId: string; me: User }) {
  const m = useMessages();
  const sync = useSync();
  const community = useCommunity(communityId);
  const access = useAccess(communityId);
  const nickname = useNickname(communityId, me.id);
  const mayChange = access?.has("changeNickname") ?? false;
  const [draft, setDraft] = useState(nickname ?? "");
  // The field follows the nickname when it changes elsewhere: another device, or a moderator.
  const [shown, setShown] = useState(nickname);
  if (shown !== nickname) {
    setShown(nickname);
    setDraft(nickname ?? "");
  }
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  if (!mayChange && nickname === undefined) {
    return null;
  }
  const save = (value: string | null) => {
    setPending(true);
    setError(null);
    sync.setNickname(communityId, value).then(
      () => {
        setPending(false);
        toast(value === null ? m.profile.nicknameCleared : m.profile.nicknameSaved);
      },
      (e: unknown) => {
        setPending(false);
        setError(problemText(e));
      },
    );
  };
  const trimmed = draft.trim();
  return (
    <Form
      className="flex flex-col gap-2"
      onSubmit={(event) => {
        event.preventDefault();
        if (trimmed !== "" && trimmed !== nickname) {
          save(trimmed);
        }
      }}
    >
      <TextField
        value={draft}
        onChange={(value) => {
          setDraft(value);
          setError(null);
        }}
        isDisabled={!mayChange}
        className={fieldClass}
      >
        <Label className={labelClass}>
          {format(m.profile.nicknameIn, { community: community?.name ?? "" })}
        </Label>
        <Input
          className={inputClass}
          placeholder={format(m.profile.nicknamePlaceholder, { name: displayNameOf(me) })}
        />
        <p className={hintClass}>{mayChange ? m.profile.nicknameHint : m.profile.nicknameLocked}</p>
      </TextField>
      {error !== null && (
        <p role="alert" className={alertClass}>
          {error}
        </p>
      )}
      <div className="flex flex-wrap gap-2">
        {mayChange && (
          <Button
            type="submit"
            isDisabled={pending || trimmed === "" || trimmed === nickname}
            className={primaryButtonClass}
          >
            {m.profile.saveNickname}
          </Button>
        )}
        {nickname !== undefined && (
          <Button
            isDisabled={pending}
            onPress={() => {
              save(null);
            }}
            className={secondaryButtonClass}
          >
            {m.profile.clearMyNickname}
          </Button>
        )}
      </div>
    </Form>
  );
}

/**
 * Clears another member's nickname, for those who may (`useMayClearNickname`); shows nothing for
 * anyone else, or when the member has none.
 */
export function ClearNicknameButton({
  communityId,
  userId,
  name,
  onError,
}: {
  communityId: string;
  userId: string;
  /** What the member is called, for the button's label. */
  name: string;
  onError: (message: string | null) => void;
}) {
  const m = useMessages();
  const sync = useSync();
  const nickname = useNickname(communityId, userId);
  const may = useMayClearNickname(communityId, userId);
  const [pending, setPending] = useState(false);
  if (!may || nickname === undefined) {
    return null;
  }
  return (
    <Button
      aria-label={format(m.profile.clearNicknameLabel, { name })}
      isDisabled={pending}
      onPress={() => {
        setPending(true);
        onError(null);
        sync.clearNickname(communityId, userId).then(
          () => {
            setPending(false);
            toast(m.profile.nicknameCleared);
          },
          (e: unknown) => {
            setPending(false);
            onError(problemText(e));
          },
        );
      }}
      className={secondaryButtonClass}
    >
      {m.profile.clearNickname}
    </Button>
  );
}

/** Reports another member's nickname in the community, while they have one. */
export function ReportNicknameButton({
  communityId,
  userId,
}: {
  communityId: string;
  userId: string;
}) {
  const m = useMessages();
  const nickname = useNickname(communityId, userId);
  if (nickname === undefined) {
    return null;
  }
  return (
    <DialogTrigger>
      <Button
        aria-label={format(m.reports.reportNicknameLabel, { nickname })}
        className={secondaryButtonClass + " flex items-center justify-center gap-1.5"}
      >
        <FlagIcon size={16} aria-hidden="true" />
        {m.reports.reportNickname}
      </Button>
      <ReportModal target={{ kind: "nickname", communityId, userId, nickname }} />
    </DialogTrigger>
  );
}
