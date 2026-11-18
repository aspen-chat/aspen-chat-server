import { EMPTY_PUSH_STATE, type PushState } from "@aspen/protocol";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

type Receive = "prompt" | "prompt-with-rationale" | "granted" | "denied";

const system = {
  receive: "prompt" as Receive,
  /** What the system answers when asked. */
  answer: "granted" as Receive,
  asked: 0,
  registered: 0,
  unregistered: 0,
};
let saved: PushState = EMPTY_PUSH_STATE;
const relayCalls: string[] = [];

vi.mock("@capacitor/push-notifications", () => ({
  PushNotifications: {
    addListener: () => Promise.resolve({ remove: () => Promise.resolve() }),
    checkPermissions: () => Promise.resolve({ receive: system.receive }),
    requestPermissions: () => {
      system.asked += 1;
      system.receive = system.answer;
      return Promise.resolve({ receive: system.answer });
    },
    register: () => {
      system.registered += 1;
      return Promise.resolve();
    },
    unregister: () => {
      system.unregistered += 1;
      return Promise.resolve();
    },
  },
}));
vi.mock("./pushBridge", () => ({
  AspenPush: {
    describe: () =>
      Promise.resolve({
        platform: "apns",
        app: "com.aspen-chat.client",
        environment: "sandbox",
        relay: "https://relay.example",
      }),
    saveState: ({ state }: { state: string }) => {
      saved = JSON.parse(state) as PushState;
      return Promise.resolve();
    },
  },
  loadState: () => Promise.resolve(saved),
}));
vi.mock("./everywhere", () => ({ useSources: () => [] }));
vi.mock("@/config", () => ({ detectShell: () => "mobile" }));
vi.mock("@tanstack/react-router", () => {
  const navigate = () => undefined;
  return { useNavigate: () => navigate };
});

let root: Root | null = null;

beforeEach(() => {
  vi.resetModules();
  window.localStorage.clear();
  document.body.replaceChildren();
  Object.assign(system, { receive: "prompt", answer: "granted", asked: 0, registered: 0 });
  system.unregistered = 0;
  saved = EMPTY_PUSH_STATE;
  relayCalls.length = 0;
  vi.stubGlobal("fetch", (input: RequestInfo | URL, init?: RequestInit) => {
    relayCalls.push(
      `${init?.method ?? "GET"} ${new URL(input instanceof Request ? input.url : input).pathname}`,
    );
    return Promise.resolve(new Response(null, { status: 204 }));
  });
});

afterEach(() => {
  act(() => {
    root?.unmount();
  });
  root = null;
  vi.unstubAllGlobals();
});

async function settle() {
  for (let i = 0; i < 10; i += 1) {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
  }
}

/** Starts the app's push as a fresh start would, with this install's stored answer. */
async function start() {
  const { WakeThisPhone } = await import("./push");
  const choice = await import("./pushChoice");
  const container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  act(() => {
    root?.render(<WakeThisPhone />);
  });
  await settle();
  return choice;
}

function button(name: string): HTMLElement {
  const found = Array.from(document.querySelectorAll<HTMLElement>("button")).find(
    (b) => b.textContent === name || b.getAttribute("aria-label") === name,
  );
  if (found === undefined) {
    throw new Error(`no button ${name}`);
  }
  return found;
}

function press(name: string) {
  act(() => {
    const target = button(name);
    target.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
    target.dispatchEvent(new PointerEvent("pointerup", { bubbles: true }));
    target.click();
  });
}

const prompt = "Enable Aspen push notifications for new messages?";

describe("WakeThisPhone", () => {
  it("asks before the system does, and asks the system only on Accept", async () => {
    const choice = await start();
    expect(document.body.textContent).toContain(prompt);
    expect(system.asked).toBe(0);
    press("Accept");
    await settle();
    expect(system.asked).toBe(1);
    expect(system.registered).toBe(1);
    expect(choice.pushStatus().choice).toBe("on");
    expect(window.localStorage.getItem("aspen.push")).toBe("on");
    expect(document.body.textContent).not.toContain(prompt);
  });

  it("on Decline asks the system nothing, and stays off on later starts", async () => {
    const choice = await start();
    press("Decline");
    await settle();
    expect(system.asked).toBe(0);
    expect(system.registered).toBe(0);
    expect(choice.pushStatus().choice).toBe("off");
    act(() => {
      root?.unmount();
    });
    root = null;
    vi.resetModules();
    await start();
    expect(document.body.textContent).not.toContain(prompt);
    expect(system.asked).toBe(0);
    expect(system.registered).toBe(0);
  });

  it("leaves the question for the next start when it is closed unanswered", async () => {
    const choice = await start();
    press("Close");
    await settle();
    expect(document.body.textContent).not.toContain(prompt);
    expect(system.asked).toBe(0);
    expect(choice.pushStatus().choice).toBeNull();
    expect(window.localStorage.getItem("aspen.push")).toBeNull();
  });

  it("keeps push on for an install the system already allowed, without asking", async () => {
    system.receive = "granted";
    const choice = await start();
    expect(document.body.textContent).not.toContain(prompt);
    expect(choice.pushStatus().choice).toBe("on");
    expect(system.registered).toBe(1);
  });

  it("does not ask when the system already refused, and says so", async () => {
    system.receive = "denied";
    const choice = await start();
    expect(document.body.textContent).not.toContain(prompt);
    expect(choice.pushStatus()).toMatchObject({ choice: null, denied: true, available: true });
  });

  it("ends a phone's registration when turned off, and registers it again when turned on", async () => {
    system.receive = "granted";
    window.localStorage.setItem("aspen.push", "on");
    saved = {
      version: 1,
      device: { id: "d1", secret: "s1", token: "t1" },
      accounts: [],
    };
    const choice = await start();
    expect(system.registered).toBe(1);
    act(() => {
      choice.choosePush("off");
    });
    await settle();
    expect(system.unregistered).toBe(1);
    expect(relayCalls).toEqual(["DELETE /v1/devices/d1"]);
    expect(saved).toEqual(EMPTY_PUSH_STATE);
    act(() => {
      choice.choosePush("on");
    });
    await settle();
    expect(system.registered).toBe(2);
  });

  it("asks the system again when turned on after it was refused and then allowed", async () => {
    window.localStorage.setItem("aspen.push", "on");
    system.receive = "denied";
    const choice = await start();
    expect(choice.pushStatus().denied).toBe(true);
    expect(system.registered).toBe(0);
    system.receive = "granted";
    act(() => {
      choice.choosePush("on");
    });
    await settle();
    expect(choice.pushStatus().denied).toBe(false);
    expect(system.registered).toBe(1);
  });
});
