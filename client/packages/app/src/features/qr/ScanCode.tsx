import { QrCodeIcon } from "@phosphor-icons/react";
import { useState } from "react";
import { Button } from "react-aria-components";
import { useMessages } from "@/i18n/context";
import { parseAspenLink, type AspenLink } from "./aspenLinks";
import { QrScannerDialog } from "./QrScanner";

/**
 * A button that opens the camera to scan one of Aspen's QR codes. `accept` takes what the code
 * leads to and returns `null` once it has acted on it, or what to tell the reader about a code
 * that is not the kind wanted here; a code that is not Aspen's at all is answered for it.
 */
export function ScanCodeButton({
  label,
  hint,
  accept,
  className,
}: {
  label: string;
  hint: string;
  accept: (link: AspenLink) => string | null;
  className: string;
}) {
  const m = useMessages();
  const [open, setOpen] = useState(false);
  return (
    <>
      <Button
        onPress={() => {
          setOpen(true);
        }}
        className={className}
      >
        <QrCodeIcon size={18} aria-hidden="true" />
        {label}
      </Button>
      {open && (
        <QrScannerDialog
          isOpen
          onOpenChange={setOpen}
          title={label}
          hint={hint}
          onScan={(text) => {
            const link = parseAspenLink(text);
            return link === null ? m.qr.notAspen : accept(link);
          }}
        />
      )}
    </>
  );
}
