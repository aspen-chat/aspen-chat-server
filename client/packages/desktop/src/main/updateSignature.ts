import { parseDn } from "builder-util-runtime";
import { execFile } from "node:child_process";

/**
 * Whether a downloaded update is signed by one of the publishers the installed build names
 * (`publisherName` in its `app-update.yml`, which electron-builder writes from
 * `azureSignOptions`), checked with Windows' own Authenticode verification. electron-updater's
 * own check lets an update through unchecked when PowerShell cannot run; this one refuses it on
 * every way of failing, so an update that cannot be checked is never installed.
 *
 * Resolves to `null` when the update may be installed and to the reason otherwise, as
 * electron-updater's `verifyUpdateCodeSignature` expects.
 */
export function verifyUpdateSignature(publishers: string[], file: string): Promise<string | null> {
  return new Promise((resolve) => {
    execFile(
      "powershell.exe",
      [
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        // The file's path reaches PowerShell through the environment, never through the
        // command's text, so no path can be read as code.
        "$signature = Get-AuthenticodeSignature -LiteralPath $env:ASPEN_UPDATE_FILE; " +
          "@{ status = [int]$signature.Status; subject = $signature.SignerCertificate.Subject } " +
          "| ConvertTo-Json -Compress",
      ],
      {
        timeout: 60_000,
        windowsHide: true,
        env: { ...process.env, ASPEN_UPDATE_FILE: file },
      },
      (error, stdout) => {
        if (error !== null) {
          resolve(`its signature could not be checked: ${error.message}`);
          return;
        }
        resolve(signatureVerdict(stdout, publishers));
      },
    );
  });
}

/** `SignatureStatus.Valid`. */
const VALID = 0;

/**
 * The verdict on what the check printed: `null` when the signature is valid and its signer
 * matches a publisher, every part of whose distinguished name must then be the signer's (a bare
 * name is a common name alone).
 */
export function signatureVerdict(output: string, publishers: readonly string[]): string | null {
  let raw: unknown;
  try {
    raw = JSON.parse(output);
  } catch {
    return `its signature could not be read: ${output}`;
  }
  if (typeof raw !== "object" || raw === null || !("status" in raw) || !("subject" in raw)) {
    return `its signature could not be read: ${output}`;
  }
  const { status, subject } = raw;
  if (status !== VALID) {
    return `its signature is not valid (status ${String(status)})`;
  }
  if (typeof subject !== "string") {
    return "it names no signer";
  }
  const signer = parseDn(subject);
  for (const publisher of publishers) {
    const named = parseDn(publisher);
    const wanted = named.size > 0 ? named : new Map([["CN", publisher]]);
    if ([...wanted].every(([key, value]) => signer.get(key) === value)) {
      return null;
    }
  }
  return `it is signed by ${subject}, not by ${publishers.join(" or ")}`;
}
