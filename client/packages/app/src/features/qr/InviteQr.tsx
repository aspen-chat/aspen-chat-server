import { QrCode, QrDownload } from "./QrCode";

/**
 * An invite's QR code and the way to save it: what is shown whenever an invite is made or looked
 * at, whatever kind it is. `link` is the invite's shareable link (`shareUrl`), which the code
 * holds; `fileName` names the saved file, without its extension.
 */
export function InviteQr({
  link,
  label,
  fileName,
}: {
  link: string;
  label: string;
  fileName: string;
}) {
  return (
    <div className="flex flex-col items-center gap-2">
      <QrCode text={link} label={label} size={176} />
      <QrDownload text={link} label={label} fileName={fileName} />
    </div>
  );
}
