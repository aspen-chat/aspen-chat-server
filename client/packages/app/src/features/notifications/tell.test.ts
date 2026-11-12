import { describe, expect, it, vi } from "vitest";
import { messageBody } from "./messageBody";
import { tell, type Described, type Shown, type Telling } from "./tell";

function telling(over: Partial<Telling> = {}) {
  const shown: Shown[] = [];
  const chime = vi.fn();
  const to: Telling = {
    quiet: () => false,
    chime,
    shows: () => true,
    show: (notification) => {
      shown.push(notification);
    },
    ...over,
  };
  return { to, shown, chime };
}

const open = () => undefined;
const described = (body: Described["body"]): Described => ({ title: "Ann", body, tag: "t", open });

describe("tell", () => {
  it("chimes and shows a notification described at once", () => {
    const { to, shown, chime } = telling();
    tell(to, () => described("hello"));
    expect(chime).toHaveBeenCalledTimes(1);
    expect(shown).toEqual([{ title: "Ann", body: "hello", tag: "t", open }]);
  });

  it("describes nothing, and so reads nothing, when nothing may tell", () => {
    const { to, shown, chime } = telling({ quiet: () => true });
    const describe = vi.fn(() => described("hello"));
    tell(to, describe);
    expect(describe).not.toHaveBeenCalled();
    expect(chime).not.toHaveBeenCalled();
    expect(shown).toEqual([]);
  });

  it("chimes but describes nothing, and so reads nothing, when no notification is shown", () => {
    const { to, shown, chime } = telling({ shows: () => false });
    const read = vi.fn(() => Promise.resolve("from the deployment"));
    tell(to, () => described(messageBody(read, "own")));
    expect(chime).toHaveBeenCalledTimes(1);
    expect(read).not.toHaveBeenCalled();
    expect(shown).toEqual([]);
  });

  it("shows a body that was waited for once it comes", async () => {
    const { to, shown } = telling();
    const read = vi.fn(() => Promise.resolve("from the deployment"));
    tell(to, () => described(messageBody(read, "own")));
    expect(shown).toEqual([]);
    await vi.waitFor(() => {
      expect(shown).toEqual([{ title: "Ann", body: "from the deployment", tag: "t", open }]);
    });
    expect(read).toHaveBeenCalledTimes(1);
  });

  it("shows nothing when the user came to look, or the message went, during the wait", async () => {
    let quiet = false;
    const { to, shown } = telling({ quiet: () => quiet });
    const body = Promise.resolve("late");
    tell(to, () => described(body));
    quiet = true;
    await body;
    await Promise.resolve();
    expect(shown).toEqual([]);

    const gone = telling();
    const nothing = Promise.resolve(null);
    tell(gone.to, () => described(nothing));
    await nothing;
    await Promise.resolve();
    expect(gone.shown).toEqual([]);
  });
});
