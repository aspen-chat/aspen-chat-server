#!/usr/bin/env bash
# Runs the Playwright specs in Playwright's own Docker image, which carries every browser and the
# libraries they need; WebKit in particular has no supported build for most Linux distributions.
# Arguments go to `playwright test`, for example `--project=phone-webkit`.
#
# The image's tag is the installed Playwright's version, so the two cannot disagree. The dev
# server runs inside the container, from the installed Vite: pnpm is not used there, since its
# dependency check would try to reinstall the host's node_modules for the container's Node.
set -euo pipefail
client="$(cd "$(dirname "$0")/.." && pwd)"
version="$(node -p "require('$client/packages/app/node_modules/@playwright/test/package.json').version")"
exec docker run --rm --init -v "$client":/client -w /client/packages/app \
  "mcr.microsoft.com/playwright:v${version}-noble" bash -c '
    ./node_modules/.bin/vite --port 5173 --strictPort > /tmp/vite.log 2>&1 &
    for _ in $(seq 60); do curl -s -o /dev/null http://localhost:5173/ && break; sleep 1; done
    exec ./node_modules/.bin/playwright test "$@"' playwright "$@"
