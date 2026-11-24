import { BrowserWindow, app, ipcMain } from "electron";
import { NsisUpdater, type UpdateInfo } from "electron-updater";
import { readFileSync, writeFileSync } from "node:fs";
import { writeFile } from "node:fs/promises";
import { join } from "node:path";
import downloads from "../../downloads.json";
import signing from "../../signing.json";
import { WINDOWS_X64 } from "../release/releases.ts";
import { isAppPage } from "./navigation";
import { defaultChoice, mayInstall, parseChoice, type UpdateChoice } from "./updateChoice.ts";
import { readUpdateLock, type UpdateLock } from "./updatePolicy.ts";
import { verifyUpdateSignature } from "./updateSignature.ts";

/**
 * The Windows build updating itself (`docs/architecture/desktop-updates.md`): electron-updater,
 * reading the feed of the track the reader follows (`latest.yml` or `beta.yml` under
 * `windows/x64/` of the downloads site), downloads a newer build in the background, checks that
 * it is signed by a publisher `signing.json` takes updates from, and installs it as the app quits, or at
 * once when the reader restarts from the notice the page shows.
 *
 * Only a build packaged to update itself does (`electron-builder.signed.mjs` marks its
 * `package.json`): every other build, and every other platform, is never given an updater. Nor
 * is a build whose updating is turned off, by the reader (`updates.json` in the profile), by an
 * administrator's policy, or by the installer's flag (`updatePolicy.ts`): with it off nothing is
 * fetched from the downloads site at all.
 */

/** How long after starting the first check waits, so it does not slow the window's first load. */
const FIRST_CHECK_MS = 30_000;
/** How often a running app checks again. */
const CHECK_INTERVAL_MS = 4 * 60 * 60 * 1000;

type Status = "idle" | "checking" | "downloading" | "ready" | "failed";

/** What the page is told: `null` from `updates:state` where this build never updates itself. */
interface UpdateState {
  choice: UpdateChoice;
  /** Who turned updating off from outside the app, which the reader's choice cannot undo. */
  locked: Exclude<UpdateLock, null> | null;
  status: Status;
  /** The version waiting to be installed while `ready`. */
  version: string | null;
}

/** Whether this build was packaged to update itself, which only the signed build is. */
function updatesItself(): boolean {
  if (process.platform !== "win32" || !app.isPackaged) {
    return false;
  }
  try {
    const raw: unknown = JSON.parse(readFileSync(join(app.getAppPath(), "package.json"), "utf8"));
    return (
      typeof raw === "object" &&
      raw !== null &&
      "aspen" in raw &&
      typeof raw.aspen === "object" &&
      raw.aspen !== null &&
      "updates" in raw.aspen &&
      raw.aspen.updates === true
    );
  } catch (error) {
    console.error("the app's package.json could not be read, so it does not update itself", error);
    return false;
  }
}

function choiceFile(): string {
  return join(app.getPath("userData"), "updates.json");
}

function readChoice(): UpdateChoice {
  try {
    const raw: unknown = JSON.parse(readFileSync(choiceFile(), "utf8"));
    if (typeof raw === "object" && raw !== null && "choice" in raw) {
      const choice = parseChoice(raw.choice);
      if (choice !== null) {
        return choice;
      }
    }
  } catch (error) {
    if (!(error instanceof Error && "code" in error && error.code === "ENOENT")) {
      console.error("the update choice could not be read", error);
    }
  }
  return defaultChoice(app.getVersion());
}

/**
 * The feed's address: the downloads site's, or `ASPEN_UPDATE_FEED`'s, which points an installed
 * build at a test feed (`publishWindows.ts --prefix`). The signature an update must carry is the same
 * either way.
 */
function feedUrl(): string {
  return process.env.ASPEN_UPDATE_FEED ?? `${downloads.url}/${WINDOWS_X64.folder}`;
}

/**
 * Writes the updater's configuration, which electron-updater would otherwise read from the
 * `app-update.yml` electron-builder writes beside the app, so the feed and the publishers come
 * from this code's own constants and cannot be missing from a package. JSON is YAML too.
 */
function writeUpdateConfig(): string {
  const file = join(app.getPath("userData"), "app-update.yml");
  writeFileSync(
    file,
    JSON.stringify({
      provider: "generic",
      url: feedUrl(),
      publisherName: signing.windows.updatesFrom,
      // electron-builder's name for it, which its uninstaller clears.
      updaterCacheDirName: "aspen-updater",
    }),
  );
  return file;
}

class Updates {
  #choice = readChoice();
  #lock: UpdateLock = null;
  #status: Status = "idle";
  #version: string | null = null;
  #updater: NsisUpdater | null = null;
  #timer: NodeJS.Timeout | null = null;
  /** Whether the lock has been read, before which no updater is made whatever the choice. */
  #started = false;
  #keeping = Promise.resolve();

  state(): UpdateState {
    return {
      choice: this.#choice,
      locked: this.#lock,
      status: this.#status,
      version: this.#version,
    };
  }

  #announce(): void {
    const state = this.state();
    for (const window of BrowserWindow.getAllWindows()) {
      window.webContents.send("updates:state", state);
    }
  }

  #set(status: Status, version: string | null = this.#version): void {
    this.#status = status;
    this.#version = version;
    this.#announce();
  }

  async start(): Promise<void> {
    this.#lock = await readUpdateLock();
    this.#started = true;
    this.#announce();
    this.#follow();
  }

  /** Starts, retunes, or stops the updater for the current choice and lock. */
  #follow(): void {
    if (!this.#started) {
      return;
    }
    if (this.#lock !== null || this.#choice === "off") {
      this.#stop();
      return;
    }
    const updater = (this.#updater ??= this.#create());
    // Setting a channel allows downgrades, which would move someone leaving beta back to an
    // older stable build; they wait for a newer one instead.
    updater.channel = this.#choice === "beta" ? "beta" : "latest";
    updater.allowDowngrade = false;
    // A download waiting for another track installs only if this one takes it
    // (`update-downloaded`, on the next check).
    updater.autoInstallOnAppQuit =
      this.#version !== null && mayInstall(this.#choice, this.#version);
    if (!updater.autoInstallOnAppQuit) {
      this.#set("idle", null);
    }
    if (this.#timer === null) {
      this.#timer = setTimeout(() => {
        this.#check();
        this.#timer = setInterval(() => {
          this.#check();
        }, CHECK_INTERVAL_MS);
      }, FIRST_CHECK_MS);
    } else {
      this.#check();
    }
  }

  #stop(): void {
    if (this.#timer !== null) {
      clearTimeout(this.#timer);
      clearInterval(this.#timer);
      this.#timer = null;
    }
    if (this.#updater !== null) {
      this.#updater.autoInstallOnAppQuit = false;
    }
    this.#set("idle", null);
  }

  #create(): NsisUpdater {
    const updater = new NsisUpdater();
    updater.updateConfigPath = writeUpdateConfig();
    updater.verifyUpdateCodeSignature = verifyUpdateSignature;
    updater.autoDownload = true;
    updater.on("checking-for-update", () => {
      if (this.#status !== "ready") {
        this.#set("checking");
      }
    });
    updater.on("update-available", () => {
      this.#set("downloading");
    });
    updater.on("update-not-available", () => {
      if (this.#status !== "ready") {
        this.#set("idle", null);
      }
    });
    updater.on("update-downloaded", (info: UpdateInfo) => {
      if (mayInstall(this.#choice, info.version)) {
        updater.autoInstallOnAppQuit = true;
        this.#set("ready", info.version);
      }
    });
    updater.on("error", (error: Error) => {
      console.error("updating failed", error);
      if (this.#status !== "ready") {
        this.#set("failed", null);
      }
    });
    return updater;
  }

  #check(): void {
    this.#updater?.checkForUpdates().catch((error: unknown) => {
      // Reported through the updater's `error` event as well.
      console.error("the update check failed", error);
    });
  }

  async choose(choice: UpdateChoice): Promise<void> {
    if (choice === this.#choice) {
      return;
    }
    this.#choice = choice;
    this.#follow();
    this.#announce();
    this.#keeping = this.#keeping.then(async () => {
      try {
        await writeFile(choiceFile(), JSON.stringify({ choice }));
      } catch (error) {
        console.error("the update choice could not be kept", error);
      }
    });
    await this.#keeping;
  }

  restart(): void {
    if (this.#status === "ready" && this.#updater !== null) {
      // Silently, as an update installs on quitting, and opening the app again afterwards.
      this.#updater.quitAndInstall(true, true);
    }
  }
}

/** Answers the app's own page alone: other frames may hold a plugin's view. */
export function serveUpdates(page: Parameters<typeof isAppPage>[1]): void {
  const updates = updatesItself() ? new Updates() : null;
  const fromApp = (event: Electron.IpcMainInvokeEvent | Electron.IpcMainEvent) =>
    event.senderFrame !== null &&
    event.senderFrame.parent === null &&
    isAppPage(event.senderFrame.url, page);
  ipcMain.handle("updates:state", (event) =>
    fromApp(event) && updates !== null ? updates.state() : null,
  );
  ipcMain.handle("updates:choose", async (event, raw: unknown) => {
    const choice = parseChoice(raw);
    if (fromApp(event) && updates !== null && choice !== null) {
      await updates.choose(choice);
    }
  });
  ipcMain.on("updates:restart", (event) => {
    if (fromApp(event)) {
      updates?.restart();
    }
  });
  if (updates !== null) {
    void updates.start();
  }
}
