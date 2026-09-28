import { renderToStaticMarkup } from "react-dom/server";
import ReactMarkdown from "react-markdown";
import { describe, expect, it } from "vitest";
import { remarkMentions } from "@/features/messages/remarkMentions";

const A = "01a0e95a-8b0f-75df-a00b-29a4f0b878d1";

const render = (markdown: string) =>
  renderToStaticMarkup(<ReactMarkdown remarkPlugins={[remarkMentions]}>{markdown}</ReactMarkdown>);

describe("remarkMentions", () => {
  it("marks people, roles, and everyone", () => {
    const html = render(`hi <@${A}>, <@&${A}> and @everyone`);
    expect(html).toContain(`<span data-mention="user" data-id="${A}">&lt;@${A}&gt;</span>`);
    expect(html).toContain(`<span data-mention="role" data-id="${A}">&lt;@&amp;${A}&gt;</span>`);
    expect(html).toContain(`<span data-mention="everyone" data-id="">@everyone</span>`);
  });

  it("leaves code, words, and malformed tags alone", () => {
    expect(render(`\`<@${A}>\` mail@everyone.example @everyones <@nope>`)).not.toContain(
      "data-mention",
    );
  });
});
