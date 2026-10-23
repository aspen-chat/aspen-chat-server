// Runs Prettier over the client and over every Markdown file the repository tracks outside it,
// passing its arguments on (`--write` or `--check`). Git names the Markdown files rather than a
// glob over the repository root, which would also reach the worktrees checked out inside it.
// Every file takes the repository root's `.prettierrc`.
import { execFileSync } from "node:child_process";
import { createRequire } from "node:module";

const markdown = execFileSync(
  "git",
  ["ls-files", "-z", "--", ":(top,glob)**/*.md", ":(top,exclude)client/**"],
  { encoding: "utf8" },
)
  .split("\0")
  .filter(Boolean);

const prettier = createRequire(import.meta.url).resolve("prettier/bin/prettier.cjs");
try {
  execFileSync(process.execPath, [prettier, ...process.argv.slice(2), ".", ...markdown], {
    stdio: "inherit",
  });
} catch (error) {
  process.exit(error.status ?? 1);
}
