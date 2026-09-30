import { ShieldCheckIcon } from "@phosphor-icons/react";
import { Button, Dialog, DialogTrigger, Modal, ModalOverlay } from "react-aria-components";
import { usePasskeyTransport } from "@/features/auth/passkeyTransport";
import {
  dialogClass,
  overlayClass,
  planesModalClass,
  secondaryButtonClass,
} from "@/features/invites/dialog";
import { DialogHeading } from "@/features/layout/DialogHeading";
import { useMessages } from "@/i18n/context";
import { SecurityPanel } from "./SecurityPanel";

/** Opens the security settings from the settings dialog. */
export function SecurityDialog() {
  const m = useMessages();
  const transport = usePasskeyTransport();
  return (
    <DialogTrigger>
      <Button className={secondaryButtonClass + " flex items-center gap-1.5 self-start"}>
        <ShieldCheckIcon size={16} aria-hidden="true" />
        {m.security.open}
      </Button>
      <ModalOverlay isDismissable className={overlayClass}>
        <Modal className={planesModalClass}>
          <Dialog className={dialogClass}>
            <DialogHeading>{m.security.title}</DialogHeading>
            <SecurityPanel transport={transport} />
          </Dialog>
        </Modal>
      </ModalOverlay>
    </DialogTrigger>
  );
}
