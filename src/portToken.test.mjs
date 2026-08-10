// Tests for the zero-import leaf module `portToken.ts` — plan 043. Imported by its `.ts` path
// directly: Node v24's built-in type-stripping runs this with no transpiler and no dependency,
// exactly like `dragGeometry.test.mjs`.
import test from "node:test";
import assert from "node:assert/strict";
import { rewritePortToken } from "./portToken.ts";

test("fresh npm command + Vite gets --port and --strictPort after one --", () => {
  const result = rewritePortToken("npm run dev", 5176, "--port", "Vite");
  assert.equal(result, "npm run dev -- --port 5176 --strictPort");
});

test("an existing --port token is replaced in place, never a second --", () => {
  const result = rewritePortToken(
    "npm run dev --workspace web -- --port 5175",
    5176,
    "--port",
    "Vite",
  );
  assert.equal(result, "npm run dev --workspace web -- --port 5176 --strictPort");
  assert.equal(result.split(" -- ").length - 1, 1);
});

test("pnpm needs no -- separator", () => {
  const result = rewritePortToken("pnpm dev", 5176, "--port", "Vite");
  assert.equal(result, "pnpm dev --port 5176 --strictPort");
});

test("Next uses -p, not --port, and no --strictPort", () => {
  const result = rewritePortToken("next dev", 5176, "--port", "Next");
  assert.equal(result, "next dev -p 5176");
});

test("mkdir -p tmp before && is never touched, even though -p means port for Next", () => {
  const result = rewritePortToken("mkdir -p tmp && npm run dev", 5176, "--port", "Next");
  assert.equal(result, "mkdir -p tmp && npm run dev -- -p 5176");
});

test("an existing PORT= is replaced, not duplicated", () => {
  const result = rewritePortToken("PORT=3000 npm start", 5176, "PORT=");
  assert.equal(result, "PORT=5176 npm start");
});

test("a command with no recognisable token gets one appended in the right place", () => {
  const result = rewritePortToken("yarn dev", 5176, "--port", "Astro");
  assert.equal(result, "yarn dev --port 5176");
});

test("CRA has no port flag, so it gets a PORT= prefix instead", () => {
  const result = rewritePortToken("npm start", 3001, "--port", "CRA");
  assert.equal(result, "PORT=3001 npm start");
});

test("unknown framework falls back to whichever button was pressed", () => {
  const flagForm = rewritePortToken("node server.js", 4001, "--port", undefined);
  assert.equal(flagForm, "node server.js --port 4001");
  const envForm = rewritePortToken("node server.js", 4001, "PORT=", undefined);
  assert.equal(envForm, "PORT=4001 node server.js");
});

test("set PORT=N on Windows is replaced whole, not split by the bare PORT= pattern", () => {
  const result = rewritePortToken("set PORT=3000 && npm run dev", 5176, "PORT=");
  assert.equal(result, "set PORT=5176 && npm run dev");
});

test("--strictPort is idempotent across a second press", () => {
  const once = rewritePortToken("npm run dev", 5175, "--port", "Vite");
  const twice = rewritePortToken(once, 5176, "--port", "Vite");
  assert.equal(twice, "npm run dev -- --port 5176 --strictPort");
});
