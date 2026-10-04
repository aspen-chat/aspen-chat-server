import { DeviceMobileIcon } from "@phosphor-icons/react";
import { Button, Dialog, DialogTrigger, Modal, ModalOverlay } from "react-aria-components";
import { useAspenClient } from "@/api/context";
import { detectShell } from "@/config";
import { dialogClass, modalClass, overlayClass } from "@/features/invites/dialog";
import { DialogHeading } from "@/features/layout/DialogHeading";
import { ScanCodeButton } from "@/features/qr/ScanCode";
import { useOpenAspenLink } from "@/features/qr/openLink";
import { useMessages } from "@/i18n/context";
import { DeviceLinkCode } from "./DeviceLinkCode";

/**
 * Signing in one device from another by a QR code. A computer (the desktop app or a browser)
 * shows a code: signed out, to be signed in by a phone; signed in, to sign a phone in. A phone
 * scans one, whichever way round. Phones do not show codes, and computers do not scan them.
 */
export function OtherDeviceSignIn({ className }: { className: string }) {
  const m = useMessages();
  const client = useAspenClient();
  const open = useOpenAspenLink();
  const signedIn = client.session !== null;
  if (detectShell() === "mobile") {
    return (
      <ScanCodeButton
        label={m.deviceLink.scan}
        hint={signedIn ? m.deviceLink.scanHintSignedIn : m.deviceLink.scanHintSignedOut}
        accept={(link) => {
          if (link.kind !== "deviceLink") {
            return m.deviceLink.notSignInCode;
          }
          open(link);
          return null;
        }}
        className={className}
      />
    );
  }
  const label = signedIn ? m.deviceLink.offer : m.deviceLink.request;
  return (
    <DialogTrigger>
      <Button className={className}>
        <DeviceMobileIcon size={18} aria-hidden="true" />
        {label}
      </Button>
      <ModalOverlay className={overlayClass} isDismissable>
        <Modal className={modalClass}>
          <Dialog className={dialogClass}>
            <DialogHeading>{label}</DialogHeading>
            <DeviceLinkCode />
          </Dialog>
        </Modal>
      </ModalOverlay>
    </DialogTrigger>
  );
}
