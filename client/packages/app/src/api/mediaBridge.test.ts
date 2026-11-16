import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

/** What the native side was asked, in order. */
const calls: { method: string; options: unknown }[] = [];
let offered = { photo: true, video: false, library: true };
interface Item {
  url: string;
  name: string;
  mimeType: string;
  byteSize: number;
  takenAt?: number;
}
let answer: { items: Item[] } = { items: [] };
let refusal: Error | null = null;

vi.mock("@capacitor/core", () => ({
  Capacitor: {
    isNativePlatform: () => true,
    isPluginAvailable: (name: string) => name === "AspenMedia",
    convertFileSrc: (url: string) => url.replace("file://", "https://localhost/_capacitor_file_"),
  },
  registerPlugin: () =>
    new Proxy(
      {},
      {
        get: (_, method: string) => (options: unknown) => {
          calls.push({ method, options });
          if (method === "sources") {
            return Promise.resolve(offered);
          }
          if (method === "choose") {
            return refusal === null ? Promise.resolve(answer) : Promise.reject(refusal);
          }
          return Promise.resolve();
        },
      },
    ),
}));

const { chooseMedia, isCameraDenied, mediaSources } = await import("./mediaBridge");

/** The ranges each read asked for, by URL. */
const ranges = new Map<string, string | null>();

/** Serves `bodies` by URL, as a range answer, and refuses any other. */
function serve(bodies: Record<string, string>) {
  vi.stubGlobal(
    "fetch",
    vi.fn((url: string, init?: RequestInit) => {
      ranges.set(url, new Headers(init?.headers).get("Range"));
      return Promise.resolve(
        url in bodies
          ? new Response(bodies[url], { status: 206 })
          : new Response(null, { status: 404 }),
      );
    }),
  );
}

describe("mediaBridge", () => {
  beforeEach(() => {
    calls.length = 0;
    ranges.clear();
    answer = { items: [] };
    refusal = null;
  });
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("offers just the sources the device says it has, asking once", async () => {
    offered = { photo: true, video: false, library: true };
    expect([...(await mediaSources())].sort()).toEqual(["library", "photo"]);
    offered = { photo: true, video: true, library: true };
    expect([...(await mediaSources())].sort()).toEqual(["library", "photo"]);
    expect(calls.filter((c) => c.method === "sources")).toHaveLength(1);
  });

  it("reads what was chosen, with its name or when it was taken, and releases every copy", async () => {
    const taken = Date.UTC(2026, 10, 2, 9, 30);
    answer = {
      items: [
        {
          url: "file:///cache/media/a/beach.jpg",
          name: "beach.jpg",
          mimeType: "image/jpeg",
          byteSize: 10,
        },
        {
          url: "file:///cache/media/b/capture.mp4",
          name: "",
          mimeType: "video/mp4",
          byteSize: 9,
          takenAt: taken,
        },
      ],
    };
    serve({
      "https://localhost/_capacitor_file_/cache/media/a/beach.jpg": "jpeg bytes",
      "https://localhost/_capacitor_file_/cache/media/b/capture.mp4": "mp4 bytes",
    });
    const chosen = await chooseMedia("library", 3);
    expect(calls[0]).toEqual({ method: "choose", options: { source: "library", limit: 3 } });
    expect(chosen.map(({ file, named }) => [file.name, file.type, named])).toEqual([
      ["beach.jpg", "image/jpeg", true],
      ["", "video/mp4", false],
    ]);
    expect(chosen[1]?.takenAt.getTime()).toBe(taken);
    expect(await chosen[0]?.file.text()).toBe("jpeg bytes");
    // Read whole, as a range, which Capacitor's file server on iOS answers for every type.
    expect([...ranges.values()]).toEqual(["bytes=0-", "bytes=0-"]);
    expect(calls.filter((c) => c.method === "release").map((c) => c.options)).toEqual([
      { url: "file:///cache/media/a/beach.jpg" },
      { url: "file:///cache/media/b/capture.mp4" },
    ]);
  });

  it("fails when a copy cannot be read, and still releases them all", async () => {
    answer = {
      items: [
        {
          url: "file:///cache/media/a/one.jpg",
          name: "one.jpg",
          mimeType: "image/jpeg",
          byteSize: 5,
        },
        {
          url: "file:///cache/media/b/two.jpg",
          name: "two.jpg",
          mimeType: "image/jpeg",
          byteSize: 5,
        },
      ],
    };
    serve({ "https://localhost/_capacitor_file_/cache/media/a/one.jpg": "bytes" });
    await expect(chooseMedia("library", 2)).rejects.toThrow("404");
    expect(calls.filter((c) => c.method === "release")).toHaveLength(2);
  });

  it("fails when a copy is read short of its length", async () => {
    answer = {
      items: [
        { url: "file:///cache/media/a/clip.mp4", name: "", mimeType: "video/mp4", byteSize: 9 },
      ],
    };
    serve({ "https://localhost/_capacitor_file_/cache/media/a/clip.mp4": "mp4" });
    await expect(chooseMedia("video", 1)).rejects.toThrow("3 bytes of 9");
    expect(calls.filter((c) => c.method === "release")).toHaveLength(1);
  });

  it("answers nothing when the user backed out", async () => {
    serve({});
    expect(await chooseMedia("photo", 1)).toEqual([]);
  });

  it("tells a refused camera from other failures", async () => {
    refusal = Object.assign(new Error("the camera is not allowed"), { code: "cameraDenied" });
    const denied = await chooseMedia("photo", 1).catch((error: unknown) => error);
    expect(isCameraDenied(denied)).toBe(true);
    expect(isCameraDenied(new Error("no camera app"))).toBe(false);
    expect(isCameraDenied(null)).toBe(false);
  });
});
