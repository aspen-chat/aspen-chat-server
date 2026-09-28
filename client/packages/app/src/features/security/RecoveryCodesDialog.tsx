import { CopyIcon, DownloadSimpleIcon } from "@phosphor-icons/react";
import { useState } from "react";
import { Button, Dialog, Heading, Modal, ModalOverlay } from "react-aria-components";
import { primaryButtonClass } from "@/features/auth/styles";
import {
  dialogClass,
  headingClass,
  modalClass,
  overlayClass,
  secondaryButtonClass,
} from "@/features/invites/dialog";
import { useMessages } from "@/i18n/context";
import { copyText } from "@/features/layout/clipboard";

/**
 * Shows a fresh set of recovery codes once. It closes only through "I've saved them", so the
 * codes are not lost to a stray click outside.
 */
export function RecoveryCodesDialog({
  codes,
  onClose,
}: {
  codes: readonly string[] | null;
  onClose: () => void;
}) {
  const m = useMessages();
  const [copied, setCopied] = useState(false);
  const text = (codes ?? []).join("\n");
  return (
    <ModalOverlay
      isOpen={codes !== null}
      isDismissable={false}
      isKeyboardDismissDisabled
      className={overlayClass}
    >
      <Modal className={modalClass}>
        <Dialog className={dialogClass} aria-label={m.security.codesHeading}>
          <Heading slot="title" className={headingClass}>
            {m.security.codesHeading}
          </Heading>
          <p className="text-sm text-ink-muted">{m.security.codesPrompt}</p>
          <ul
            aria-label={m.security.recoveryHeading}
            className="grid grid-cols-2 gap-x-6 gap-y-1 rounded-md bg-surface px-4 py-3 font-mono text-sm"
          >
            {(codes ?? []).map((code) => (
              <li key={code}>{code}</li>
            ))}
          </ul>
          <div className="flex flex-wrap items-center justify-between gap-2">
            <div className="flex gap-2">
              <Button
                onPress={(event) => {
                  void copyText(text, event.target).then((done) => {
                    // Where copying is impossible the codes are on screen, and can be downloaded.
                    setCopied(done);
                  });
                }}
                className={secondaryButtonClass + " flex items-center gap-1.5"}
              >
                <CopyIcon size={14} aria-hidden="true" />
                {copied ? m.security.copied : m.security.copy}
              </Button>
              <Button
                onPress={() => {
                  const link = document.createElement("a");
                  link.href = URL.createObjectURL(new Blob([text + "\n"], { type: "text/plain" }));
                  link.download = m.security.downloadName;
                  link.click();
                  URL.revokeObjectURL(link.href);
                }}
                className={secondaryButtonClass + " flex items-center gap-1.5"}
              >
                <DownloadSimpleIcon size={14} aria-hidden="true" />
                {m.security.download}
              </Button>
            </div>
            <Button
              onPress={() => {
                setCopied(false);
                onClose();
              }}
              className={primaryButtonClass}
            >
              {m.security.savedThem}
            </Button>
          </div>
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}
