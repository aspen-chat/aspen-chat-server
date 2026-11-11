import { describe, expect, it } from "vitest";
import { en } from "@/i18n/messages";
import { held } from "./describe";

describe("held", () => {
  it("says what a message of files alone holds", () => {
    expect(held(en, [])).toBe("New message");
    expect(held(en, ["image/png"])).toBe("Sent an image");
    expect(held(en, ["image/png", "Image/JPEG"])).toBe("Sent images");
    expect(held(en, ["application/pdf"])).toBe("Sent an attachment");
    expect(held(en, ["image/png", "video/mp4"])).toBe("Sent an attachment");
  });

  it("takes a file it does not hold for one that is not a picture", () => {
    expect(held(en, ["image/png", undefined])).toBe("Sent an attachment");
  });
});
