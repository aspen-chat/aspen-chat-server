import { describe, expect, it } from "vitest";
import spec from "../../../../../../spec/web_client_page.json";
import { embeddedProfile, PROFILE_ELEMENT_ID } from "./deploymentProfile";

const profile = {
  displayName: "Gardeners",
  icon: { id: "0190", mimeType: "image/webp", downloadUrl: "https://media.example.org/i" },
  webClientUrl: "https://chat.example.org",
};

function page(text: string | null): Document {
  const doc = document.implementation.createHTMLDocument("");
  if (text !== null) {
    const script = doc.createElement("script");
    script.type = "application/json";
    script.id = PROFILE_ELEMENT_ID;
    script.textContent = text;
    doc.head.append(script);
  }
  return doc;
}

describe("embeddedProfile", () => {
  it("names the element as the server writes it", () => {
    expect(PROFILE_ELEMENT_ID).toBe(spec.profileElementId);
  });

  it("reads the profile of the deployment serving the page", () => {
    const doc = page(JSON.stringify(profile));
    expect(embeddedProfile(doc, "https://chat.example.org")).toEqual(profile);
    expect(embeddedProfile(doc, "https://chat.example.org/")).toEqual(profile);
  });

  it("reads the escapes the server writes in place of characters that would end its element", () => {
    const doc = page(
      '{"displayName":"\\u003c/script\\u003e \\u0026","icon":null,"webClientUrl":"https://chat.example.org"}',
    );
    expect(embeddedProfile(doc, "https://chat.example.org")?.displayName).toBe("</script> &");
  });

  it("leaves out another deployment's profile, and anything that is not one", () => {
    expect(embeddedProfile(page(JSON.stringify(profile)), "https://other.example.org")).toBeNull();
    expect(embeddedProfile(page(null), "https://chat.example.org")).toBeNull();
    expect(embeddedProfile(page("not json"), "https://chat.example.org")).toBeNull();
    expect(embeddedProfile(page("null"), "https://chat.example.org")).toBeNull();
    expect(embeddedProfile(page('{"webClientUrl":7}'), "https://chat.example.org")).toBeNull();
    expect(
      embeddedProfile(page('{"webClientUrl":"nowhere"}'), "https://chat.example.org"),
    ).toBeNull();
  });
});
