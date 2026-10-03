import { ApiProblemError } from "@aspen/protocol";
import { useState } from "react";
import { Button, Dialog, Modal, ModalOverlay } from "react-aria-components";
import { useDeploymentCan, useSync } from "@/api/hooks";
import { alertClass } from "@/features/auth/styles";
import { BanFields } from "@/features/community-settings/BanFields";
import { NEW_BAN, banRequest, type BanChoice } from "@/features/community-settings/banChoice";
import {
  dangerButtonClass,
  dialogClass,
  modalClass,
  overlayClass,
} from "@/features/invites/dialog";
import { ChoiceCheckbox } from "@/features/layout/choices";
import { DialogHeading } from "@/features/layout/DialogHeading";
import { toast } from "@/features/layout/toast";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/**
 * Bans someone from the whole server: a reason they are told when they try to sign in, how
 * long for, for a holder of Moderate any community whether their messages everywhere from the
 * last hour or day go with them, and for a bot whether its owner is banned too. Done, it says
 * so in a toast and closes.
 */
export function UserBanDialog({
  userId,
  name,
  bot,
  isOpen,
  onOpenChange,
  onBanned,
}: {
  userId: string;
  name: string;
  bot: boolean;
  isOpen: boolean;
  onOpenChange: (open: boolean) => void;
  onBanned: () => void;
}) {
  const m = useMessages();
  const sync = useSync();
  const mayDelete = useDeploymentCan("moderateCommunities");
  const [choice, setChoice] = useState<BanChoice>(NEW_BAN);
  const [withOwner, setWithOwner] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function ban() {
    if (busy) {
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const outcome = await sync.admin.banUser(userId, {
        ...banRequest(choice, mayDelete),
        withOwner: bot && withOwner,
      });
      toast(
        outcome.deletedMessages > 0
          ? format(m.deployments.bannedAndDeleted, {
              name,
              count: String(outcome.deletedMessages),
            })
          : format(m.deployments.bannedNamed, { name }),
      );
      onOpenChange(false);
      onBanned();
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
          <DialogHeading>{format(m.deployments.banHeading, { name })}</DialogHeading>
          <p className="text-sm text-ink-muted">{m.deployments.banHint}</p>
          <BanFields
            value={choice}
            onChange={setChoice}
            mayDelete={mayDelete}
            reasonHint={m.deployments.banReasonHint}
            deleteLabel={m.deployments.banDeleteLabel}
          />
          {bot && (
            <ChoiceCheckbox
              isSelected={withOwner}
              onChange={setWithOwner}
              label={m.deployments.banWithOwner}
              hint={m.deployments.banWithOwnerHint}
            />
          )}
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
