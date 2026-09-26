import { ApiProblemError } from "@aspen/protocol";
import { useCallback, useId, useState, type ReactNode } from "react";
import { useSync } from "@/api/hooks";
import { CropDialog } from "@/features/media/CropDialog";
import { useMessages } from "@/i18n/context";

/**
 * Chooses a picture, has the reader crop it, uploads the result as an icon, and reports the
 * new icon's id. `children` renders the control that opens the file chooser and receives the
 * pending state; errors show below it.
 */
export function IconPicker({
  onIcon,
  children,
}: {
  onIcon: (iconId: string) => void | Promise<void>;
  children: (open: () => void, pending: boolean) => ReactNode;
}) {
  const m = useMessages();
  const sync = useSync();
  // The chooser is found by id rather than a ref so the render prop may call `open` freely.
  const inputId = useId();
  const [file, setFile] = useState<File | null>(null);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const open = useCallback(() => {
    document.getElementById(inputId)?.click();
  }, [inputId]);

  async function upload(icon: Blob) {
    setFile(null);
    setPending(true);
    setError(null);
    try {
      const record = await sync.uploadIcon(icon, "image/png");
      await onIcon(record.id);
    } catch (e) {
      setError(e instanceof ApiProblemError ? e.message : String(e));
    } finally {
      setPending(false);
    }
  }

  return (
    <>
      <input
        id={inputId}
        type="file"
        accept="image/*"
        hidden
        aria-hidden="true"
        tabIndex={-1}
        onChange={(event) => {
          const chosen = event.target.files?.[0] ?? null;
          event.target.value = "";
          if (chosen !== null) {
            setFile(chosen);
          }
        }}
      />
      {children(open, pending)}
      {error !== null && (
        <p role="alert" className="text-sm text-danger">
          {error}
        </p>
      )}
      {file !== null && (
        <CropDialog
          file={file}
          onCrop={(icon) => {
            void upload(icon);
          }}
          onCancel={() => {
            setFile(null);
          }}
        />
      )}
      {pending && <span className="text-sm text-ink-muted">{m.crop.uploading}</span>}
    </>
  );
}
