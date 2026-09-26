/**
 * Generates TypeScript from the server's machine-readable API descriptions.
 *
 * Inputs (repository root, both gitignored, both produced by
 * `cargo run -p aspen-chat-server -- --gen-openapi-schema`):
 *   - openapi.yaml        REST API
 *   - event_schema.json   JSON Schema (draft 2020-12) of the WebSocket protocol: `ClientMessage`,
 *                         `ServerMessage`, and the `ServerEvent` union they carry
 *   - voice_signal_schema.json  JSON Schema of the voice server's signalling frames, produced by
 *                         `cargo run -p voice_server -- --gen-signal-schema`
 *
 * Outputs (src/generated/, gitignored):
 *   - openapi.ts          `paths` and `components` types consumed by openapi-fetch
 *   - events.ts           `ClientMessage`, `ServerMessage`, `ServerEvent`, and the record types
 *   - event_schema.json   verbatim copy, loaded by the runtime validator in ../src/events.ts
 *   - voiceSignal.ts      `ClientMessage` and `ServerMessage` of the voice signalling protocol
 *
 * Usage: `pnpm codegen [--regen]`. When either input is missing the server is asked to produce
 * them; `--regen` forces that even when they exist so the client never builds against a stale
 * description. Producing the inputs needs a Rust toolchain; consuming them does not.
 */

import { execFileSync } from "node:child_process";
import { existsSync } from "node:fs";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { compile, type JSONSchema } from "json-schema-to-typescript";
import openapiTS, { astToString } from "openapi-typescript";

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(here, "../../../..");
const openapiPath = resolve(repoRoot, "openapi.yaml");
const eventSchemaPath = resolve(repoRoot, "event_schema.json");
const voiceSchemaPath = resolve(repoRoot, "voice_signal_schema.json");
const outDir = resolve(here, "../src/generated");

const banner = `/* eslint-disable */
/**
 * GENERATED FILE. Do not edit.
 * Produced by \`pnpm codegen\` (client/packages/protocol/scripts/codegen.ts) from the server's
 * schema output. Change the Rust types and regenerate instead.
 */
`;

function regenerateServerSchemas(): void {
  console.log("codegen: running the server's schema generator via cargo");
  execFileSync("cargo", ["run", "-p", "aspen-chat-server", "--", "--gen-openapi-schema"], {
    cwd: repoRoot,
    stdio: "inherit",
  });
  console.log("codegen: running the voice server's schema generator via cargo");
  execFileSync("cargo", ["run", "-p", "voice_server", "--", "--gen-signal-schema"], {
    cwd: repoRoot,
    stdio: "inherit",
  });
}

/**
 * The server emits draft 2020-12, where `$ref` may sit beside `properties` and every keyword
 * applies. json-schema-to-typescript predates that rule and would drop the siblings, so each
 * such object is rewritten as an equivalent `allOf` before compilation. The runtime validator
 * loads the untouched schema.
 */
function liftRefSiblings(node: unknown): unknown {
  if (Array.isArray(node)) {
    return node.map(liftRefSiblings);
  }
  if (node === null || typeof node !== "object") {
    return node;
  }
  const record = node as Record<string, unknown>;
  const rewritten: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(record)) {
    rewritten[key] = liftRefSiblings(value);
  }
  const keysBesidesRef = Object.keys(rewritten).filter(
    (k) => k !== "$ref" && k !== "description" && k !== "title",
  );
  if (typeof rewritten.$ref === "string" && keysBesidesRef.length > 0) {
    const { $ref, description, title, ...rest } = rewritten;
    const lifted: Record<string, unknown> = { allOf: [{ $ref }, rest] };
    if (description !== undefined) lifted.description = description;
    if (title !== undefined) lifted.title = title;
    return lifted;
  }
  return rewritten;
}

async function generateOpenApi(): Promise<void> {
  const ast = await openapiTS(pathToFileURL(openapiPath), {
    exportType: true,
    arrayLength: false,
    rootTypes: false,
  });
  await writeFile(resolve(outDir, "openapi.ts"), banner + astToString(ast));
}

async function generateEvents(): Promise<void> {
  const raw = await readFile(eventSchemaPath, "utf8");
  const schema = JSON.parse(raw) as JSONSchema;
  const compilable = liftRefSiblings(schema) as JSONSchema;
  const source = await compile(compilable, "EventStreamProtocol", {
    bannerComment: banner,
    additionalProperties: false,
    strictIndexSignatures: true,
    unknownAny: true,
    cwd: dirname(eventSchemaPath),
    // The schema's own `$defs` are compiled in place; nothing is fetched.
    $refOptions: { resolve: { external: false } },
  });
  await writeFile(resolve(outDir, "events.ts"), source);
  await writeFile(resolve(outDir, "event_schema.json"), raw);
}

async function generateVoiceSignal(): Promise<void> {
  const raw = await readFile(voiceSchemaPath, "utf8");
  const schema = JSON.parse(raw) as JSONSchema;
  const compilable = liftRefSiblings(schema) as JSONSchema;
  const source = await compile(compilable, "VoiceSignalProtocol", {
    bannerComment: banner,
    additionalProperties: false,
    strictIndexSignatures: true,
    unknownAny: true,
    cwd: dirname(voiceSchemaPath),
    $refOptions: { resolve: { external: false } },
  });
  await writeFile(resolve(outDir, "voiceSignal.ts"), source);
}

async function main(): Promise<void> {
  const regen = process.argv.includes("--regen");
  const missing =
    !existsSync(openapiPath) || !existsSync(eventSchemaPath) || !existsSync(voiceSchemaPath);
  if (regen || missing) {
    if (missing) {
      console.log(`codegen: ${openapiPath}, ${eventSchemaPath}, or ${voiceSchemaPath} is missing`);
    }
    try {
      regenerateServerSchemas();
    } catch (error) {
      console.error(
        "codegen: could not run the server's schema generator. Install a Rust toolchain, or " +
          "produce openapi.yaml, event_schema.json, and voice_signal_schema.json at the " +
          "repository root some other way.",
      );
      throw error;
    }
  }
  await mkdir(outDir, { recursive: true });
  await Promise.all([generateOpenApi(), generateEvents(), generateVoiceSignal()]);
  console.log(
    `codegen: wrote openapi.ts, events.ts, event_schema.json, voiceSignal.ts to ${outDir}`,
  );
}

main().catch((error: unknown) => {
  console.error(error);
  process.exitCode = 1;
});
