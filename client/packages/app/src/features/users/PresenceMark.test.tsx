import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { PresenceMark } from "./PresenceMark";

const render = (status: "online" | "away" | "offline", label?: string) =>
  renderToStaticMarkup(
    label === undefined ? (
      <PresenceMark status={status} />
    ) : (
      <PresenceMark status={status} label={label} />
    ),
  );

describe("PresenceMark", () => {
  it("draws each status in a shape of its own", () => {
    expect(render("online")).toContain('<circle cx="5" cy="5" r="5"></circle>');
    expect(render("away")).toContain("<mask");
    expect(render("offline")).toContain('fill="none"');
    const shapes = new Set((["online", "away", "offline"] as const).map((s) => render(s)));
    expect(shapes.size).toBe(3);
  });

  it("is named where it stands alone, and hidden beside text", () => {
    expect(render("away", "Away")).toContain('role="img" aria-label="Away"');
    expect(render("online")).toContain('aria-hidden="true"');
  });
});
