import { ApiProblemError, type ManaSender } from "@aspen/protocol";
import { useEffect, useState } from "react";
import { Button, Dialog, Modal, ModalOverlay } from "react-aria-components";
import { useChannel, useSync, useUser } from "@/api/hooks";
import { alertClass } from "@/features/auth/styles";
import { Avatar } from "@/features/communities/Avatar";
import { BanDialog } from "@/features/community-settings/BanDialog";
import { NEW_BAN, type BanChoice } from "@/features/community-settings/banChoice";
import { useMemberModeration } from "@/features/community-settings/roleAssignment";
import {
  dialogClass,
  listModalClass,
  overlayClass,
  secondaryButtonClass,
} from "@/features/invites/dialog";
import { DialogHeading } from "@/features/layout/DialogHeading";
import { displayNameOf } from "@/features/users/profile";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/** How often the list is read again while open, as the server sums it at most this often. */
const REFRESH_MS = 5_000;

/** A ban from this list deletes the person's messages from the past hour, which the list counts. */
const BAN_FROM_LIST: BanChoice = { ...NEW_BAN, window: "hour" };

/**
 * Who spent most of a channel's mana in the past hour (the server's `mana::top_senders`), the
 * most first, read again every `REFRESH_MS` while open. In a community, each person the caller
 * may ban (`useMemberModeration`) has a Ban button, opening the ban dialog set to delete their
 * messages from the past hour.
 */
export function TopSendersDialog({
  channelId,
  isOpen,
  onOpenChange,
}: {
  channelId: string;
  isOpen: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const m = useMessages();
  const sync = useSync();
  const channel = useChannel(channelId);
  const [senders, setSenders] = useState<ManaSender[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [banning, setBanning] = useState<{ userId: string; name: string } | null>(null);
  useEffect(() => {
    if (!isOpen) {
      return;
    }
    let live = true;
    const read = () => {
      sync
        .topManaSenders(channelId)
        .then((read) => {
          if (live) {
            setSenders(read);
            setError(null);
          }
        })
        .catch((e: unknown) => {
          if (live) {
            setError(e instanceof ApiProblemError ? e.message : String(e));
          }
        });
    };
    read();
    const every = window.setInterval(read, REFRESH_MS);
    return () => {
      live = false;
      window.clearInterval(every);
    };
  }, [isOpen, channelId, sync]);
  const communityId = channel?.community ?? null;
  return (
    <>
      <ModalOverlay
        isOpen={isOpen}
        onOpenChange={onOpenChange}
        isDismissable
        className={overlayClass}
      >
        <Modal className={listModalClass}>
          <Dialog className={dialogClass}>
            <DialogHeading>
              {format(m.mana.topSendersHeading, { channel: channel?.name ?? "" })}
            </DialogHeading>
            <p className="text-sm text-ink-muted">{m.mana.topSendersHint}</p>
            {error !== null && (
              <p role="alert" className={alertClass}>
                {error}
              </p>
            )}
            {senders === null ? (
              <p role="status" className="text-sm text-ink-muted">
                {m.mana.topSendersLoading}
              </p>
            ) : senders.length === 0 ? (
              <p className="text-sm text-ink-muted">{m.mana.topSendersNone}</p>
            ) : (
              <ol
                aria-label={m.mana.topSenders}
                className="flex max-h-[60vh] flex-col overflow-y-auto"
              >
                {senders.map((sender, index) => (
                  <SenderRow
                    key={sender.user}
                    place={index + 1}
                    sender={sender}
                    communityId={communityId}
                    onBan={(name) => {
                      setBanning({ userId: sender.user, name });
                    }}
                  />
                ))}
              </ol>
            )}
          </Dialog>
        </Modal>
      </ModalOverlay>
      {banning !== null && communityId !== null && (
        <BanDialog
          communityId={communityId}
          userId={banning.userId}
          name={banning.name}
          isOpen
          initial={BAN_FROM_LIST}
          onOpenChange={(open) => {
            if (!open) {
              setBanning(null);
            }
          }}
        />
      )}
    </>
  );
}

function SenderRow({
  place,
  sender,
  communityId,
  onBan,
}: {
  place: number;
  sender: ManaSender;
  communityId: string | null;
  onBan: (name: string) => void;
}) {
  const m = useMessages();
  const user = useUser(sender.user);
  const name = user === undefined ? m.mana.unknownSender : displayNameOf(user);
  return (
    <li className="flex items-center gap-3 border-b border-line py-2 last:border-b-0">
      <span className="w-8 shrink-0 text-end text-sm text-ink-faint tabular-nums">{place}</span>
      <Avatar name={name} iconId={user?.icon} size="xs" />
      <span className="min-w-0 flex-1 truncate">{name}</span>
      <span className="shrink-0 text-sm text-ink-muted tabular-nums">
        {format(m.mana.spent, { mana: sender.mana })}
      </span>
      {communityId !== null && (
        <BanButton
          communityId={communityId}
          userId={sender.user}
          onPress={() => {
            onBan(name);
          }}
        />
      )}
    </li>
  );
}

function BanButton({
  communityId,
  userId,
  onPress,
}: {
  communityId: string;
  userId: string;
  onPress: () => void;
}) {
  const m = useMessages();
  const { mayBan } = useMemberModeration(communityId, userId);
  if (!mayBan) {
    return null;
  }
  return (
    <Button onPress={onPress} className={secondaryButtonClass}>
      {m.mana.ban}
    </Button>
  );
}
