/**
 * Pure token rewrite for §10 step 4's "Choose for me" — SPEC.md §10 step 4, plan 043.
 *
 * ZERO IMPORTS — same hard requirement as `dragGeometry.ts`: `node --test` must reach this without
 * a transpiler, so nothing here may import `./api` or anything that chains to it. One press of
 * "Choose for me" must update the Port field AND this token in the same press — §10 step 4 says the
 * two halves are inseparable, because Hangar's `port` is a prediction of what the child binds, never
 * an instruction to it.
 */

/** What the dialog's button offers when the framework is unknown (§10 step 4). */
export type PortTokenForm = "--port" | "PORT=";

type ResolvedFlag = "--port" | "-p" | "PORT=";

/**
 * §10 step 4's flag table — exact match on `stack.framework`'s own strings (registry.rs's
 * `FRAMEWORK_DETECTORS` names) wins; anything else falls back to whichever button was pressed.
 */
const FRAMEWORK_FLAG: Record<string, ResolvedFlag> = {
  Vite: "--port",
  Astro: "--port",
  Nuxt: "--port",
  SvelteKit: "--port",
  Angular: "--port",
  Remix: "--port",
  Next: "-p",
  CRA: "PORT=",
};

function resolveFlag(framework: string | undefined, form: PortTokenForm): ResolvedFlag {
  if (framework !== undefined && framework in FRAMEWORK_FLAG) return FRAMEWORK_FLAG[framework];
  return form;
}

type PackageManager = "npm" | "pnpm" | "yarn" | "other";

/** Trap 3: derive from the command string's own first token — never the dialog's `packageManager`
 * state, which resets to `"npm"` on every dialog open and is unreliable here. */
function detectPackageManager(segment: string): PackageManager {
  const first = segment.trim().split(/\s+/)[0] ?? "";
  if (first === "npm" || first === "npm.cmd") return "npm";
  if (first === "pnpm" || first === "pnpm.cmd") return "pnpm";
  if (first === "yarn" || first === "yarn.cmd") return "yarn";
  return "other";
}

interface ExistingToken {
  text: string;
  rebuild: (port: number) => string;
}

/**
 * Recognises `--port N`, `--port=N`, `-p N`, `PORT=N`, `set PORT=N`, in that priority order so
 * `set PORT=N` (which also contains a bare `PORT=N` substring) is matched whole rather than split.
 * Searches the WHOLE command, not a segment: a Windows `set PORT=3000 && npm run dev` needs the
 * part BEFORE its `&&` found and replaced. The `-p` pattern requires a digit immediately after it,
 * so `mkdir -p tmp` (non-numeric) and a bare `pnpm -p` (its own `--parallel` shorthand) never match.
 */
function findExistingToken(command: string): ExistingToken | null {
  const eq = command.match(/--port=\d+/);
  if (eq) return { text: eq[0], rebuild: (port) => `--port=${port}` };

  const long = command.match(/--port\s+\d+/);
  if (long) return { text: long[0], rebuild: (port) => `--port ${port}` };

  const short = command.match(/(^|\s)-p\s+\d+/);
  if (short) return { text: short[0], rebuild: (port) => `${short[1]}-p ${port}` };

  const setEnv = command.match(/\bset\s+PORT=\d+/);
  if (setEnv) return { text: setEnv[0], rebuild: (port) => `set PORT=${port}` };

  const env = command.match(/\bPORT=\d+/);
  if (env) return { text: env[0], rebuild: (port) => `PORT=${port}` };

  return null;
}

/** Trap 2: append after an existing ` -- `, never emit a second one. npm needs one before
 * pass-through flags; pnpm/yarn/bare binaries (`next dev`, `vite`) do not. */
function appendFlag(segment: string, token: string, pm: PackageManager): string {
  const trimmed = segment.replace(/\s+$/, "");
  if (pm !== "npm") return `${trimmed} ${token}`;
  if (/(^|\s)--(\s|$)/.test(trimmed)) return `${trimmed} ${token}`;
  return `${trimmed} -- ${token}`;
}

/**
 * Shared shape for both mutations below: try to find-and-replace something already in `command`
 * first; only if nothing is found do we fall back to appending — and appending is scoped to the
 * segment after the last ` && ` (Trap 1: `mkdir -p tmp && npm run dev`'s `mkdir` is not "the"
 * command), with the package manager read from that same segment's first token (Trap 3).
 */
function withCommand(
  command: string,
  tryExisting: (whole: string) => string | null,
  appendTo: (segment: string, pm: PackageManager) => string,
): string {
  const existingResult = tryExisting(command);
  if (existingResult !== null) return existingResult;

  const lastAndIdx = command.lastIndexOf(" && ");
  const prefix = lastAndIdx === -1 ? "" : command.slice(0, lastAndIdx + 4);
  const segment = lastAndIdx === -1 ? command : command.slice(lastAndIdx + 4);
  return prefix + appendTo(segment, detectPackageManager(segment));
}

function rewritePortFlag(command: string, port: number, flag: ResolvedFlag): string {
  return withCommand(
    command,
    (whole) => {
      const existing = findExistingToken(whole);
      return existing ? whole.replace(existing.text, existing.rebuild(port)) : null;
    },
    (segment, pm) =>
      flag === "PORT=" ? `PORT=${port} ${segment}` : appendFlag(segment, `${flag} ${port}`, pm),
  );
}

/** Idempotent: a second press must not produce `--strictPort --strictPort`. */
function ensureStrictPort(command: string): string {
  return withCommand(
    command,
    (whole) => (/--strictPort\b/.test(whole) ? whole : null),
    (segment, pm) => appendFlag(segment, "--strictPort", pm),
  );
}

/**
 * Rewrite `command`'s port-setting token to `port`. `framework` (from `stack.framework`) picks the
 * concrete syntax per the table above; when it is absent or not in the table, `form` — whichever of
 * the dialog's two buttons was pressed — decides. Vite additionally gets `--strictPort` ensured
 * present: it turns a taken port from a 60 s wait then a killed server into a ~1 s honest crash.
 */
export function rewritePortToken(
  command: string,
  port: number,
  form: PortTokenForm,
  framework?: string,
): string {
  const flag = resolveFlag(framework, form);
  const rewritten = rewritePortFlag(command, port, flag);
  return framework === "Vite" ? ensureStrictPort(rewritten) : rewritten;
}
