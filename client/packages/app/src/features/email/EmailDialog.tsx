import { EnvelopeSimpleIcon } from "@phosphor-icons/react";
import { Button, Dialog, DialogTrigger, Modal, ModalOverlay } from "react-aria-components";
import { useEmailChanges, useMe } from "@/api/hooks";
import {
  dialogClass,
  overlayClass,
  planesModalClass,
  secondaryButtonClass,
} from "@/features/invites/dialog";
import { DialogHeading } from "@/features/layout/DialogHeading";
import { useMessages } from "@/i18n/context";
import { EmailPanel } from "./EmailPanel";
import { useEmailPolicy } from "./policy";

/**
 * Opens the account's email settings from the settings dialog, where the deployment sends email
 * and the account is a person of this deployment: a bot, or a user of another deployment, has
 * no address here.
 */
export function EmailDialog() {
  const m = useMessages();
  const me = useMe();
  const policy = useEmailPolicy();
  const changes = useEmailChanges();
  if (policy?.available !== true || me == null || me.bot || me.homeDomain != null) {
    return null;
  }
  return (
    <DialogTrigger>
      <Button className={secondaryButtonClass + " flex items-center gap-1.5 self-start"}>
        <EnvelopeSimpleIcon size={16} aria-hidden="true" />
        {m.email.open}
      </Button>
      <ModalOverlay isDismissable className={overlayClass}>
        <Modal className={planesModalClass}>
          <Dialog className={dialogClass}>
            <DialogHeading>{m.email.title}</DialogHeading>
            <EmailPanel changes={changes} />
          </Dialog>
        </Modal>
      </ModalOverlay>
    </DialogTrigger>
  );
}
