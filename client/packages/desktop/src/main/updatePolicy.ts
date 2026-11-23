import { execFile } from "node:child_process";

/**
 * Whether updating has been turned off for this install from outside the app: by an
 * administrator's policy for the machine (`DisableUpdates` under
 * `HKLM\Software\Policies\Aspen`, as Group Policy writes it), or when it was installed
 * (`DisableUpdates` under `HKCU\Software\Aspen`, which the installer's `/DISABLEUPDATES` writes,
 * `build/installer.nsh`). Either wins over the reader's own choice in Settings. A value of the
 * DWORD 0 leaves updating on; any other value, of any type, turns it off.
 */
export type UpdateLock = "administrator" | "installer" | null;

/**
 * PowerShell prints both values as JSON, `null` for one that is absent, so a missing key is told
 * apart from a failure by the output's shape rather than by an error's wording.
 */
const SCRIPT = [
  "$ErrorActionPreference = 'Stop'",
  "function Read-Value($path) {",
  "  if (-not (Test-Path -LiteralPath $path)) { return $null }",
  "  $item = Get-ItemProperty -LiteralPath $path",
  "  if ($item.PSObject.Properties.Name -notcontains 'DisableUpdates') { return $null }",
  "  $value = $item.DisableUpdates",
  '  return @{ type = $value.GetType().Name; value = "$value" }',
  "}",
  "@{",
  "  machine = Read-Value 'HKLM:\\Software\\Policies\\Aspen'",
  "  user = Read-Value 'HKCU:\\Software\\Aspen'",
  "} | ConvertTo-Json -Compress",
].join("\n");

/** One value as the script reports it: its .NET type's name and its text. */
interface Read {
  type: string;
  value: string;
}

function readOf(raw: unknown): Read | null | undefined {
  if (raw === null) {
    return null;
  }
  if (typeof raw === "object" && "type" in raw && "value" in raw) {
    const { type, value } = raw;
    if (typeof type === "string" && typeof value === "string") {
      return { type, value };
    }
  }
  return undefined;
}

/** Whether a value turns updating off: anything but the DWORD 0, which PowerShell reads as an Int32. */
function turnsOff(read: Read | null): boolean {
  return read !== null && !(read.type === "Int32" && read.value === "0");
}

/**
 * The lock the script's output describes. Output of any other shape locks, as an administrator's
 * would: a policy that cannot be read is taken to say what it may well say.
 */
export function lockOf(output: string): UpdateLock {
  let raw: unknown;
  try {
    raw = JSON.parse(output);
  } catch {
    return "administrator";
  }
  if (typeof raw !== "object" || raw === null || !("machine" in raw) || !("user" in raw)) {
    return "administrator";
  }
  const machine = readOf(raw.machine);
  const user = readOf(raw.user);
  if (machine === undefined || user === undefined) {
    return "administrator";
  }
  if (turnsOff(machine)) {
    return "administrator";
  }
  return turnsOff(user) ? "installer" : null;
}

/** Reads both values; a PowerShell that cannot run, or fails, locks as an unreadable policy does. */
export function readUpdateLock(): Promise<UpdateLock> {
  return new Promise((resolve) => {
    execFile(
      "powershell.exe",
      ["-NoProfile", "-NonInteractive", "-Command", SCRIPT],
      { timeout: 20_000, windowsHide: true },
      (error, stdout) => {
        if (error !== null) {
          console.error("the update policy could not be read, so updating stays off", error);
          resolve("administrator");
          return;
        }
        resolve(lockOf(stdout));
      },
    );
  });
}
