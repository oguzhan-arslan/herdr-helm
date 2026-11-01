// The ⋯ menu of the helm top bar: the actions of the layout in groups, each with its key from
// keys.json (the "layout:helm" section first, then "global"), the one place for the keys.
// Herdr gives mods no macOS menu bar, so this takes its place. ↑/↓ move, Enter runs, Esc closes.

import { View, div, svg } from "gpui-kit";
import { v_flex, h_flex } from "gpui-base";
import { run, props, closeOverlay, theme, keys, spaces, focused, panes, shared } from "herdr";
import { can, isAgent } from "./helm.js";

const icon = (name) => (name.includes("/") ? name : `mods/workspace-chrome/${name}.svg`);
const SYMBOL = { cmd: "⌘", shift: "⇧", alt: "⌥", ctrl: "⌃", enter: "↩", space: "Space", tab: "⇥", escape: "Esc", backspace: "⌫",
  up: "↑", down: "↓", left: "←", right: "→" };

// "cmd-shift-enter" -> "⌘⇧↩"
function pretty(chord) {
  return chord.split(" ").map((k) => k.split("-").map((p) => SYMBOL[p] ?? p.toUpperCase()).join("")).join(" ");
}

// The key bound to an action: in helm first, then everywhere.
function keyOf(all, action) {
  const k = all.find((b) => b.section === "layout:helm" && b.action === action) ?? all.find((b) => b.section === "global" && b.action === action);
  return k ? pretty(k.keys) : "";
}

// The last space before the shown one, as a quick alt-tab goes to.
function lastSpace() {
  const activity = shared("helm.activity") ?? {};
  return spaces().filter((s) => !s.active).sort((a, b) => (activity[b.id] ?? 0) - (activity[a.id] ?? 0))[0];
}

// [label, icon, action, enabled?]; null is a divider. The action is the command a key in
// keys.json runs ("helm-bar.smart", "tab.new", ...): the row runs it, and shows its key.
function items() {
  const space = spaces().find((s) => s.active);
  const terminal = focused()?.terminal;
  const tile = terminal && panes().find((p) => p.terminal === terminal);
  const agent = isAgent(tile) && can("agent", "restart");
  const label = shared("helm.connectors")?.agent?.label ?? "the agent";
  const update = shared("helm.update");
  return [
    // Only when a newer release is there. A click installs it.
    ...(update?.available ? [[shared("helm.updating") ? `Updating Helm to ${update.latest}…` : `Update Helm to ${update.latest}`, "mods/helm-bar/update.svg", "helm-bar.update", !shared("helm.updating")], null] : []),
    ["New space…", "plus", "helm-bar.new_space"],
    ["Spaces", "layers", "helm-bar.spaces"],
    ["Last space", "chevron-left", "helm-bar.switch", Boolean(lastSpace())],
    null,
    ["New terminal", "terminal", "tab.new"],
    ["New agent…", "plus", "helm-bar.agent"],
    ["Next tile", "chevron-right", "helm-bar.next", Boolean(terminal)],
    ["Make main tile", "mods/helm-tile/main.svg", "helm-bar.main", Boolean(terminal)],
    ["Full screen tile", "square", "helm-bar.zoom", Boolean(terminal)],
    ["Restart " + label + " in this tile", "mods/helm-tile/restart.svg", "helm-bar.restart", agent],
    ["Close tile", "x", "helm-bar.close", Boolean(terminal)],
    null,
    ["Smart rename space", "mods/helm-bar/sparkles.svg", "helm-bar.smart", Boolean(space)],
    ["Rename space…", "square-pen", "helm-bar.rename", Boolean(space)],
    null,
    ["Files", "files", "helm-bar.files"],
    ["Changes", "git-compare", "helm-bar.changes"],
    ...(can("code_host", "branch_prs") || can("code_host", "org_prs") ? [["GitHub", "mods/helm-bar/github.svg", "helm-bar.github"]] : []),
    ...(can("agent", "usage") ? [[`${label} usage`, "clock", "helm-bar.usage"]] : []),
    null,
    ["Helm Settings…", "settings", "helm-bar.settings"],
    ["Keys", "terminal", "app.help"],
    ["Herdr settings", "settings", "app.settings"],
  ];
}

// Runs an action the way its key does. A last space goes there at once; the bar's switch
// would open the spaces window and wait for alt to be released.
function act(action) {
  if (action === "helm-bar.switch") {
    const s = lastSpace();
    return s && run("space.select", { id: s.id });
  }
  run(action);
}

export default class HelmMenu extends View {
  init(_props, cx) {
    this.at = 0;
    this.handle = cx.focus_handle();
    this.handle.focus();
  }

  choose(i) {
    const it = items().filter(Boolean)[i];
    if (!it || it[3] === false) return;
    closeOverlay(props().overlay);
    act(it[2]);
  }

  render() {
    const t = theme();
    const all = keys();
    const list = items();
    const rows = list.filter(Boolean);
    let n = -1;
    return v_flex().id("helm-menu").track_focus(this.handle).w_full().min_h(0).p(5).bg(t.popover).text_size(12.8)
      .on_key_down((e, cx) => {
        if (e.key === "down") this.at = (this.at + 1) % rows.length;
        else if (e.key === "up") this.at = (this.at + rows.length - 1) % rows.length;
        else if (e.key === "enter") return this.choose(this.at);
        else if (e.key === "escape") return closeOverlay(props().overlay);
        else return;
        cx.stop_propagation();
        cx.notify();
      })
      .children(list.map((it, j) => {
        if (!it) return div().id(`div-${j}`).mx(7).my(4).h(1).bg(t.border);
        const i = ++n;
        const enabled = it[3] !== false;
        return h_flex().id(`fm-${i}`).h(30).px(8).gap(9).rounded(7).items_center()
          .text_color(enabled ? t.foreground : t.dim)
          .when(i === this.at && enabled, (el) => el.bg(t.selected))
          .when(enabled, (el) => el.hover((s) => s.bg(t.hover)).on_click(() => this.choose(i)))
          .child(svg(icon(it[1])).size(14).flex_none().text_color(enabled ? t.muted : t.dim))
          .child(div().flex_1().truncate().child(it[0]))
          .child(div().flex_none().text_size(11).text_color(t.dim).child(keyOf(all, it[2])));
      }));
  }
}
