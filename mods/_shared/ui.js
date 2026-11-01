// Small parts that more than one Helm mod uses: where the user works, times, paths, and a few
// views. Shared: the installer copies mods/_shared into every Helm mod. Edit it here only.

import { div, svg, image } from "gpui-kit";
import { spaces, panes, focused, home, window } from "herdr";

// Herdr in front: background reads pause while another app is active, and catch up on return.
export const awake = () => window()?.active !== false;

// The last terminal folder of each space: a mod pane in front has no folder, and the space's
// own may be empty (home).
const lastCwd = new Map();

// The focused folder: { space, cwd, on }. A mod pane in front has none: the space's last.
// `on` is the machine of the focused terminal, as exec takes it (null: this one).
export function location() {
  const space = spaces().find((s) => s.active), f = focused();
  const all = panes();
  const pane = f?.pane ? all.find((p) => p.pane === f.pane) : null;
  const on = f?.pane === null ? (f.terminal?.includes("/") ? f.terminal.split("/")[0] : null) : space?.machine ?? null;
  const own = pane?.cwd || f?.cwd;
  if (own && space) lastCwd.set(space.id, own);
  const tabs = new Set(space?.tabs.map((t) => t.id) ?? []);
  const cwd = own || lastCwd.get(space?.id) || all.find((p) => tabs.has(p.tab) && p.cwd)?.cwd || space?.folder || null;
  return { space: space?.id ?? null, cwd, on };
}

// Where space `s` is now: its first terminal's live folder, else where it was made.
export function place(s) {
  const tabs = new Set((s?.tabs ?? []).map((c) => c.id));
  return panes().find((p) => tabs.has(p.tab) && p.cwd)?.cwd || s?.folder;
}

// How long ago `time` was (ms or an ISO text), in short: "now", "5m", "3h", "2d".
export function ago(time) {
  const ms = typeof time === "string" ? Date.parse(time) : time;
  const m = Math.max(0, Math.round((Date.now() - ms) / 60000));
  return m < 1 ? "now" : m < 60 ? `${m}m` : m < 48 * 60 ? `${Math.round(m / 60)}h` : `${Math.round(m / 1440)}d`;
}

// The same as a phrase: "just now", "5m ago".
export function agoText(time) {
  const a = ago(time);
  return a === "now" ? "just now" : `${a} ago`;
}

// A path with the home folder as "~".
export function shortHome(path) {
  if (!path) return "";
  const h = home();
  return path === h ? "~" : path.startsWith(h + "/") ? "~" + path.slice(h.length) : path;
}

// The icon of a pane's program (p.program, or an agent's), or null when it has none.
export function programIcon(p, size, color) {
  if (!p?.icon) return null;
  return p.tint ? svg(p.icon).size(size).flex_none().text_color(color) : image(p.icon).w(size).h(size).flex_none();
}

// A check box, ticked when `on`.
export function checkBox(t, on) {
  return div().size(14).flex_none().rounded(4).border(1).border_color(on ? t.accent : t.input).bg(on ? t.accent : t.background)
    .flex().items_center().justify_center().text_size(10).text_color(t.background).child(on ? "✓" : "");
}
