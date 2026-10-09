import type { UserOnlineStatus } from "@aspen/protocol";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { PresenceMark } from "./PresenceMark";
import { knownStatus, showsConnected } from "./presenceStatus";

const render = (status: UserOnlineStatus, label?: string) =>
  renderToStaticMarkup(
    label === undefined ? (
      <PresenceMark status={status} />
    ) : (
      <PresenceMark status={status} label={label} />
    ),
  );

describe("PresenceMark", () => {
  it("draws each status others may see in a shape of its own", () => {
    expect(render("online")).toContain('<circle cx="5" cy="5" r="5"></circle>');
    expect(render("away")).toContain("<circle");
    expect(render("doNotDisturb")).toContain("<rect x=");
    expect(render("offline")).toContain('fill="none"');
    const shapes = new Set(
      (["online", "away", "doNotDisturb", "offline"] as const).map((s) =>
        render(s).replace(/id="[^"]*"|url\(#[^)]*\)/g, ""),
      ),
    );
    expect(shapes.size).toBe(4);
  });

  it("draws invisible as offline, which is how everyone else sees it", () => {
    expect(render("invisible")).toBe(render("offline"));
  });

  it("is named where it stands alone, and hidden beside text", () => {
    expect(render("away", "Away")).toContain('role="img" aria-label="Away"');
    expect(render("online")).toContain('aria-hidden="true"');
  });

  it("takes a status it does not know for offline", () => {
    expect(knownStatus("busy")).toBe("offline");
    expect(knownStatus("doNotDisturb")).toBe("doNotDisturb");
    expect(render("busy" as UserOnlineStatus)).toBe(render("offline"));
    expect(showsConnected("invisible")).toBe(false);
    expect(showsConnected("doNotDisturb")).toBe(true);
  });
});
