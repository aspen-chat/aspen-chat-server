import { ShieldCheckIcon } from "@phosphor-icons/react";
import { Button, Dialog, DialogTrigger, Heading, Modal, ModalOverlay } from "react-aria-components";
import { usePasskeyTransport } from "@/features/auth/passkeyTransport";
import {
  dialogClass,
  headingClass,
  modalClass,
  overlayClass,
  secondaryButtonClass,
} from "@/features/invites/dialog";
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
        <Modal className={modalClass + " max-h-[90vh] overflow-y-auto"}>
          <Dialog className={dialogClass}>
            {({ close }) => (
              <>
                <Heading slot="title" className={headingClass}>
                  {m.security.title}
                </Heading>
                <SecurityPanel transport={transport} />
                <Button onPress={close} className={secondaryButtonClass + " self-end"}>
                  {m.security.done}
                </Button>
              </>
            )}
          </Dialog>
        </Modal>
      </ModalOverlay>
    </DialogTrigger>
  );
}
