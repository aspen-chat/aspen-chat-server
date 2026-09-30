import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { formatNodes } from "./formatNodes";

describe("formatNodes", () => {
  it("puts elements where the template names them, wherever the translation places them", () => {
    expect(
      renderToStaticMarkup(
        <p>{formatNodes("Missed call from {name}.", { name: <b>@Kate</b> })}</p>,
      ),
    ).toBe("<p>Missed call from <b>@Kate</b>.</p>");
    expect(
      renderToStaticMarkup(<p>{formatNodes("{name} hat angerufen", { name: <b>@Kate</b> })}</p>),
    ).toBe("<p><b>@Kate</b> hat angerufen</p>");
  });

  it("leaves a placeholder it has no value for as written", () => {
    expect(renderToStaticMarkup(<p>{formatNodes("From {who}", {})}</p>)).toBe("<p>From {who}</p>");
  });
});
