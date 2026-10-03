import { ApiProblemError } from "@aspen/protocol";
import { useState } from "react";
import { Button, Dialog, Modal, ModalOverlay } from "react-aria-components";
import { useAccess, useCommunity, useSync } from "@/api/hooks";
import { alertClass } from "@/features/auth/styles";
import { BanFields } from "@/features/community-settings/BanFields";
import { NEW_BAN, banRequest, type BanChoice } from "@/features/community-settings/banChoice";
import {
  dangerButtonClass,
  dialogClass,
  modalClass,
  overlayClass,
} from "@/features/invites/dialog";
import { DialogHeading } from "@/features/layout/DialogHeading";
import { toast } from "@/features/layout/toast";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/**
 * Bans a member: a reason they are shown when they try to come back, how long for, and, for a
 * banner who also holds Manage messages, whether their messages from the last hour or day go
 * with them. Done, it says so in a toast and closes.
 */
export function BanDialog({
  communityId,
  userId,
  name,
  isOpen,
  onOpenChange,
}: {
  communityId: string;
  userId: string;
  name: string;
  isOpen: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const m = useMessages();
  const sync = useSync();
  const community = useCommunity(communityId);
  const access = useAccess(communityId);
  const mayDelete = access?.has("manageMessages") ?? false;
  const [choice, setChoice] = useState<BanChoice>(NEW_BAN);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function ban() {
    if (busy) {
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const deleted = await sync.banMember(communityId, userId, banRequest(choice, mayDelete));
      toast(
        deleted > 0
          ? format(m.members.bannedAndDeleted, { name, count: String(deleted) })
          : format(m.members.banned, { name }),
      );
      onOpenChange(false);
    } catch (e) {
      setError(e instanceof ApiProblemError ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <ModalOverlay
      isOpen={isOpen}
      onOpenChange={onOpenChange}
      isDismissable
      className={overlayClass}
    >
      <Modal className={modalClass}>
        <Dialog role="alertdialog" className={dialogClass}>
          <DialogHeading>{format(m.members.banHeading, { name })}</DialogHeading>
          <p className="text-sm text-ink-muted">
            {format(m.members.banHint, { community: community?.name ?? "" })}
          </p>
          <BanFields
            value={choice}
            onChange={setChoice}
            mayDelete={mayDelete}
            reasonHint={m.members.banReasonHint}
            deleteLabel={m.members.banDeleteLabel}
          />
          {error !== null && (
            <p role="alert" className={alertClass}>
              {error}
            </p>
          )}
          <div className="flex justify-end">
            <Button
              isDisabled={busy}
              onPress={() => {
                void ban();
              }}
              className={dangerButtonClass}
            >
              {busy ? m.members.banning : m.members.banNow}
            </Button>
          </div>
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}
