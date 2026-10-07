import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { formatNodes, listNodes } from "./formatNodes";

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

describe("listNodes", () => {
  it("joins elements as the language lists things", () => {
    const names = [<b key="a">Ana</b>, <b key="b">Ben</b>, <b key="c">Cy</b>];
    expect(renderToStaticMarkup(<p>{listNodes("en", names.slice(0, 1))}</p>)).toBe(
      "<p><b>Ana</b></p>",
    );
    expect(renderToStaticMarkup(<p>{listNodes("en", names.slice(0, 2))}</p>)).toBe(
      "<p><b>Ana</b> and <b>Ben</b></p>",
    );
    expect(renderToStaticMarkup(<p>{listNodes("en", names)}</p>)).toBe(
      "<p><b>Ana</b>, <b>Ben</b>, and <b>Cy</b></p>",
    );
    expect(renderToStaticMarkup(<p>{listNodes("de", names)}</p>)).toBe(
      "<p><b>Ana</b>, <b>Ben</b> und <b>Cy</b></p>",
    );
  });
});
