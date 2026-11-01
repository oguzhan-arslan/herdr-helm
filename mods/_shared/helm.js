// Shared by the Helm mods: calls to herdr-helm, the Helm program in the Herdr home.
// A mod cannot import from another mod's folder, so the installer copies mods/_shared into
// every Helm mod. Edit it here only.

import { exec, appHome, readTextFile, shared, setShared, openOverlay, notify } from "herdr";

export const HELM = () => `${appHome()}/helm/bin/herdr-helm`;
export const STATE = () => `${appHome()}/helm/state`;

// The last line of stderr, without the program's prefix.
const why = (text) => ((text || "").trim().split("\n").pop() ?? "").replace(/^herdr-helm: /, "");

// Mods run outside a terminal, so the guard against agents does not apply: no HC_TERMINAL_ID.
const argv = (args) => ["-u", "HC_TERMINAL_ID", HELM(), "--json", ...args];

function answer(code, stdout, stderr) {
  let data = null;
  try { data = JSON.parse(stdout ?? ""); } catch {}
  return { code, ok: code === 0, data, error: code === 0 ? "" : why(stderr) || `exit ${code}` };
}

// `herdr-helm --json <args>` for a quick command: { code, ok, data, error }.
export async function helm(args) {
  const r = await exec("env", argv(args)).catch((e) => ({ code: -1, stdout: "", stderr: String(e) }));
  return answer(r.code, r.stdout, r.stderr);
}

// Runs argv in the background, for a command that can take longer than exec's 30 s limit, and
// waits for it in short steps: { code, stdout, stderr }. It never throws. `cwd` is the folder of
// the command, `on` the machine (as exec has it). `onLine` gets the last stderr line on each
// step. The output goes to helm/state/jobs/, or to a temp folder on another machine.
export async function background(argv, { cwd = "", on = null, onLine = null } = {}) {
  const dir = on ? "" : `${STATE()}/jobs`;
  const name = `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 7)}`;
  const at = 'd=${1:-${TMPDIR:-/tmp}/helm-jobs}; f="$d/$2"';
  const run = (script, ...rest) => exec("sh", ["-c", `${at}; ${script}`, "sh", dir, name, ...rest], { on })
    .catch((e) => ({ code: -1, stdout: "", stderr: String(e) }));
  const start = 'c=$3; shift 3; mkdir -p "$d" || exit 2; find "$d" -type f -mmin +1440 -delete 2>/dev/null; (cd "${c:-.}" && "$@" > "$f.out" 2> "$f.err"; echo $? > "$f.code") </dev/null >/dev/null 2>&1 &';
  const started = await run(start, cwd, ...argv);
  if (started.code !== 0) return { code: -1, stdout: "", stderr: started.stderr || "The job did not start." };
  // Exit 1: not done yet. It prints the last stderr line.
  const wait = 'for i in $(seq 1 5); do [ -f "$f.code" ] && exit 0; sleep 0.4; done; tail -n 1 "$f.err" 2>/dev/null; exit 1';
  for (;;) {
    const w = await run(wait);
    if (w.code === 0) break;
    if (w.code !== 1) return { code: -1, stdout: "", stderr: w.stderr || "The job stopped answering." };
    const line = (w.stdout ?? "").trim();
    if (onLine && line) onLine(line);
  }
  const read = (x) => (on ? run(`cat "$f${x}"`).then((r) => (r.code === 0 ? r.stdout : null)) : readTextFile(`${dir}/${name}${x}`).catch(() => null));
  const [code, stdout, stderr] = await Promise.all([".code", ".out", ".err"].map(read));
  await run('rm -f "$f.code" "$f.out" "$f.err"');
  return { code: parseInt(code ?? "1", 10), stdout: stdout ?? "", stderr: stderr ?? "" };
}

// `herdr-helm --json <args>` in the background (see background): { code, ok, data, error }.
export async function helmSlow(args, onLine) {
  const r = await background(["env", ...argv(args)], { onLine });
  return answer(r.code, r.stdout, r.stderr);
}

// Writes a shared value only when it changed: each write makes every mod render again.
export function keep(key, value) {
  if (JSON.stringify(value) !== JSON.stringify(shared(key))) setShared(key, value);
}

// The icon of the agent connector, for the usage meter: svg path and color. Without one, a
// plain meter in the accent color.
export function agentIcon(t) {
  const icon = shared("helm.connectors")?.agent?.icon;
  return { path: icon?.path ?? "mods/helm-bar/usage.svg", color: icon?.color ?? t.accent };
}

// Whether pane `p` runs the agent of the agent connector: Herdr's app id, or the program.
export function isAgent(p) {
  const a = shared("helm.connectors")?.agent;
  if (!p || !a) return false;
  return p.app === a.name || (a.programs ?? []).includes(p.program?.key);
}

// Restarts the agent in `terminal` of space `space` (`herdr-helm restart`), for the tile's button
// and the key. When a new terminal takes the place of the old one, the tile order follows.
// It returns the answer's data, or null after a warning.
export async function restartAgent(terminal, space) {
  const r = await helm(["restart", "--terminal", terminal]);
  const d = r.ok ? r.data : null;
  const order = shared("helm.order") ?? {};
  if (d?.new && space && order[space]) setShared("helm.order", { ...order, [space]: order[space].map((x) => (x === terminal ? d.new : x)) });
  if (!d) notify({ title: "Restart failed", body: r.error, level: "warning" });
  return d;
}

// What the chosen connectors can do (`herdr-helm connectors`, which helm-jobs puts in shared
// "helm.connectors"). The UI hides what a connector cannot do. Before the first answer: all.
export function can(kind, capability) {
  const c = shared("helm.connectors");
  if (!c) return true;
  return (c[kind]?.capabilities ?? []).includes(capability);
}

// Every overlay the Helm mods open, with its size: the one place for these numbers. A mod.json
// "overlay" repeats its entry, for when Herdr opens the mod by itself. tests/mods.rs makes
// sure that they match. "mod" is the mod, when the name is not the mod.
export const OVERLAYS = {
  "helm-task": { kind: "dialog", width: 520, height: 640 },
  "helm-task-end": { mod: "helm-task", kind: "dialog", width: 460, height: 260 },
  "helm-projects": { kind: "dialog", width: 860, height: 620 },
  "helm-menu": { kind: "popup", width: 300, height: 760 },
  "helm-spaces": { kind: "popup", width: 980, height: 620 },
  "helm-usage": { kind: "popup", width: 300, height: 90 },
  "helm-usage-system": { mod: "helm-usage", kind: "popup", width: 460, height: 470 },
  "github": { kind: "popup", width: 620, height: 760 },
  "new-menu": { kind: "popup", width: 250, height: 512 },
  "worktree": { kind: "dialog", width: 440, height: 260 },
  "worktree-open": { mod: "worktree", kind: "dialog", width: 560, height: 480 },
};

// Opens overlay `name` of OVERLAYS with these props. `more` adds or replaces fields, such as a
// width that depends on the window, or a layout instead of a mod.
export function overlay(name, props = {}, more = {}) {
  const { mod = name, ...size } = OVERLAYS[name];
  openOverlay({ ...(more.layout ? {} : { mod }), ...size, props, ...more });
}
