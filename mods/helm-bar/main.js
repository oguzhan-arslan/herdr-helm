// The top bar of the helm layout, and its tiler.
// Bar: the overview button, the actions of the shown space, the shown space (its title, project, PR, agents), how many other
// spaces need you, then CPU and memory, the agent's usage, GitHub, Changes, Files, and the ⋯ menu.
// The parts a connector cannot serve do not show (helm.js can()).
// Tiler: the "grid" node of the layout shows every terminal of the shown space as a tile. The
// main tile takes the left half; the others share the right half, in more rows than columns
// (at most one more). A tile can fill the grid alone (shared "helm.zoom": space -> terminal),
// and the order of the tiles is yours (shared "helm.order": drag a title bar onto another, or
// Make main). The tabs stay as they are, hidden.
// It also stamps when each space last did something (shared "helm.activity"), for the switcher.
// A timer does the tiling, the stamps, and the side panel: render writes nothing.
// Key commands: "zoom" (the focused tile, full or back), "close" (the tab of the focused tile),
// "switch" and "switch_back" (the switcher), "spaces" (the overview), "github" (the popup),
// "next" and "prev" (the keyboard to the next or previous tile), "tile1" to "tile9".

import { View, div, svg } from "gpui-kit";
import { h_flex } from "gpui-base";
import { actionButtons, lookAtCheckout } from "./space-actions.js";
import { helm, helmSlow, overlay, OVERLAYS, can, isAgent, restartAgent, keep, agentIcon } from "./helm.js";
import { awake } from "./ui.js";
import { spaces, panes, agents, focused, run, theme, window, shared, setShared, layout, settings, nextCommand, notify } from "herdr";

const LAYOUT = "helm";
function find(node, id) {
  if (node?.id === id) return node;
  for (const child of node?.children ?? []) {
    const found = find(child, id);
    if (found) return found;
  }
  return null;
}

// The terminals of a space in the order you put them (shared "helm.order": space -> terminals,
// by drag and Make main). New ones go last: agents, then the rest, each in tab order.
function tilesOf(space, ps) {
  if (!space) return [];
  const tabs = new Map(space.tabs.map((t, i) => [t.id, i]));
  const mine = ps.filter((p) => p.terminal && tabs.has(p.tab)).sort((a, b) => tabs.get(a.tab) - tabs.get(b.tab));
  const list = [...mine.filter((p) => p.program?.agent), ...mine.filter((p) => !p.program?.agent)].map((p) => p.terminal);
  const saved = (shared("helm.order") ?? {})[space.id] ?? [];
  const kept = saved.filter((x) => list.includes(x));
  return [...kept, ...list.filter((x) => !kept.includes(x))];
}

// A drop: tile a takes the place of tile b and b the place of a. With b null, a goes to place 1.
function moved(list, a, b) {
  if (!list.includes(a)) return list;
  if (b === null) return [a, ...list.filter((x) => x !== a)];
  if (!list.includes(b)) return list;
  return list.map((x) => (x === a ? b : x === b ? a : x));
}

// A tile: its header, and its terminal. While a tile is dragged, a drop area takes the place of
// each terminal (a mod sees no mouse events over a terminal); the terminals keep running.
// `at`: the tile's number, as cmd-1 to cmd-9 use it.
function tile(terminal, id, at, dropping) {
  return {
    id: `cell-${id}`, type: "column", radius: 10, background: "background",
    border: { width: 1, color: "border" }, focus: { border: { width: 1, color: "accent" } }, resizable: false,
    children: [
      { id: `head-${id}`, type: "block", size: { px: 30 }, content: { mod: "helm-tile", props: { terminal, main: id === "main", at } } },
      dropping
        ? { id: `term-${id}`, type: "block", content: { mod: "helm-tile", props: { drop: terminal, at } } }
        : { id: `term-${id}`, type: "block", content: { terminal } },
    ],
  };
}

// The right half is tall: more rows than columns, at most one more (2 tiles stack, 5 are 3 rows).
// A host argument may nest at most 16 levels: a column of single tiles has no row wrappers.
function side(list, at, dropping) {
  const cols = Math.ceil(list.length / Math.ceil(Math.sqrt(list.length)));
  const rows = [];
  for (let i = 0; i < list.length; i += cols) rows.push(list.slice(i, i + cols));
  const children = cols === 1
    ? rows.map((row, r) => tile(row[0], `${r}-0`, at(row[0]), dropping))
    : rows.map((row, r) => ({ id: `side-${r}`, type: "row", gap: 8, children: row.map((term, c) => tile(term, `${r}-${c}`, at(term), dropping)) }));
  return { id: "side", type: "column", gap: 8, children };
}

// The children of the grid (a row): the main tile, then the right half.
function stage(list, at, dropping) {
  if (!list.length) return [{ id: "empty", type: "block", content: { mod: "helm-tile", props: { empty: true } } }];
  return [tile(list[0], "main", at(list[0]), dropping), ...(list.length > 1 ? [side(list.slice(1), at, dropping)] : [])];
}

// What a grid shows, and its shape. Herdr saves the layout with its own key order and
// defaults, so compare these and not the saved JSON.
const sig = (n) => [n.id, n.type, n.content?.terminal ?? n.content?.mod ?? null, n.content?.props?.main ?? null,
  n.content?.props?.at ?? null, n.content?.props?.drop ?? null, (n.children ?? []).map(sig)];
const ids = (n) => [n.id, (n.children ?? []).map(ids)];

function sizes(n, out = {}) {
  if (n.size) out[n.id] = n.size;
  for (const c of n.children ?? []) sizes(c, out);
  return out;
}

function resize(n, from) {
  if (from[n.id]) n.size = from[n.id];
  for (const c of n.children ?? []) resize(c, from);
}

function level(pct, t) {
  return pct >= 80 ? t.ansi1 : pct >= 60 ? t.ansi3 : t.muted;
}

function openPanel(id) {
  run("layout.select", { stack: "panel", child: id });
  run("layout.show", { id: "panel" });
}

export default class Bar extends View {
  init(_props, cx) {
    this.panels = JSON.parse(localStorage.getItem("panels") ?? "{}");
    this.activity = { ...(shared("helm.activity") ?? {}) };
    this.seen = {}; // space -> its tabs' statuses at the last look
    this.system = null; // { cpu, mem, total }
    this.moveSeen = shared("helm.move")?.at ?? 0;
    // The tiler, the activity stamps, and the side panel. Render must not write shared values or
    // the layout, so this timer does it. It is fast because a drag shows its drop areas from
    // here, and a new or closed terminal must get its tile at once.
    cx.timer.every(250, (cx) => {
      this.tile();
      const sp = spaces();
      this.stamp(sp);
      // The panel is the stack itself: Herdr folds a node with one child into that child.
      const panel = find(layout(), "panel");
      this.syncPanel(sp.find((s) => s.active), panel, panel, shared("workspace.panels") ?? ["files", "changes"]);
      const id = sp.find((s) => s.active)?.id;
      if (id) lookAtCheckout(id, cx);
    });
    this.readSystem(cx);
    cx.timer.every(5000, (cx) => awake() && this.readSystem(cx));
    // The details for the switcher and the overview, so that opening them runs nothing.
    cx.timer.after(4000, (cx) => this.readInfo(cx));
    cx.timer.every(30000, (cx) => awake() && this.readInfo(cx));
    cx.spawn(async (cx) => {
      for (;;) {
        const c = await nextCommand();
        if (c === "zoom") this.zoom(focused()?.terminal);
        if (c === "close") this.closeFocused();
        if (c === "github") this.openGithub();
        if (c === "spaces") this.openSpaces(null);
        if (c === "switch" || c === "switch_back") this.switch(c === "switch" ? 1 : -1);
        if (c === "next" || c === "prev") this.focusTile(c === "next" ? 1 : -1);
        if (/^tile[1-9]$/.test(c)) this.focusTile(0, Number(c.slice(4)) - 1);
        this.command(c);
      }
    });
  }

  // The other key commands of keys.json "layout:helm"; the ⋯ menu runs them too.
  command(c) {
    const space = spaces().find((s) => s.active);
    const terminal = focused()?.terminal;
    if (c === "smart" && space) setShared("smart.request", { id: space.id, at: Date.now() });
    if (c === "rename" && space) run("space.rename", { id: space.id });
    if (c === "main" && space && terminal) setShared("helm.move", { space: space.id, a: terminal, b: null, at: Date.now() });
    // The same checks as the restart button of the tile.
    if (c === "restart" && terminal && space && isAgent(panes().find((p) => p.terminal === terminal)) && can("agent", "restart")) restartAgent(terminal, space.id);
    if (c === "new_space") overlay("helm-task", { mode: "new" });
    if (c === "agent") overlay("new-menu");
    if (c === "files" || c === "changes") {
      const panel = find(layout(), "panel");
      if (panel && !panel.hidden && panel.active === c) run("layout.hide", { id: "panel" });
      else openPanel(c);
    }
    if (c === "usage") this.openUsage();
    if (c === "settings") overlay("helm-projects");
    if (c === "menu") overlay("helm-menu");
    if (c === "update") this.update();
  }

  // Installs the newest release, only when the person asks (the ⋯ menu). The install replaces
  // the mods, and Herdr starts them again.
  update() {
    const u = shared("helm.update");
    if (!u?.available || shared("helm.updating")) return;
    setShared("helm.updating", true);
    notify({ title: `Updating Helm to ${u.latest}…`, body: "Herdr reloads the mods when it is done.", level: "info" });
    helmSlow(["update"]).then((r) => {
      setShared("helm.updating", false);
      if (r.ok) setShared("helm.update", { ...u, available: false });
      notify(r.ok ? { title: `Helm ${u.latest} is installed`, body: "", level: "success" } : { title: "The update failed", body: r.error, level: "warning" });
    });
  }

  openUsage() {
    if (!can("agent", "usage")) return;
    const limits = (shared("agent.usage")?.limits ?? []).filter((l) => l.kind !== "weekly_scoped");
    overlay("helm-usage", {}, { height: OVERLAYS["helm-usage"].height + limits.length * 46 });
  }

  // Sets the grid of the shown space when it differs from what the layout shows.
  // layout.set writes the shown layout: never while another layout shows.
  tile() {
    if (shared("layout") !== LAYOUT) return;
    const root = layout();
    let grid = find(root, "grid");
    // Herdr removes the block of an ended terminal, then folds each container left with one
    // child into that child. When the last tile closes, the grid folds into the tile's header
    // block and its id is gone. Put a new grid back in front of the panel, without the rest.
    if (!grid) {
      const body = find(root, "work-body");
      if (!body) return;
      grid = { id: "grid", type: "row", gap: 8, children: [] };
      body.children = [grid, ...(body.children ?? []).filter((c) => c.id === "panel")];
    }
    const space = spaces().find((s) => s.active);
    const drag = shared("helm.drag");
    if (drag && Date.now() - drag.at > 10000) setShared("helm.drag", null);
    const dropping = Boolean(drag?.active) && drag.space === space?.id;
    let list = tilesOf(space, panes());
    const move = shared("helm.move");
    if (move && move.at !== this.moveSeen) {
      this.moveSeen = move.at;
      if (move.space === space?.id) {
        list = moved(list, move.a, move.b);
        keep("helm.order", { ...(shared("helm.order") ?? {}), [space.id]: list });
      }
    }
    // The tile numbers for other mods (the GitHub popup), only when they change.
    const tiles = JSON.stringify({ space: space?.id ?? null, list });
    if (tiles !== this.tiles) { this.tiles = tiles; setShared("helm.tiles", JSON.parse(tiles)); }
    const zoomed = (shared("helm.zoom") ?? {})[space?.id];
    const at = (term) => list.indexOf(term) + 1;
    const want = stage(zoomed && list.includes(zoomed) ? [zoomed] : list, at, dropping);
    if (grid.type === "row" && JSON.stringify(grid.children.map(sig)) === JSON.stringify(want.map(sig))) return;
    // The same shape keeps the sizes that were dragged.
    if (JSON.stringify(grid.children.map(ids)) === JSON.stringify(want.map(ids))) {
      const old = sizes({ children: grid.children });
      want.forEach((n) => resize(n, old));
    }
    grid.type = "row";
    grid.children = want;
    // A new grid moves the keyboard to another block: give it back to the same terminal, also
    // after a drag (the drop areas hide the terminals).
    // While the drop areas show, the focus is on no terminal or a wrong one: keep the one from
    // before the drag.
    if (!this.dropping) this.keep = focused()?.terminal ?? this.keep;
    this.dropping = dropping;
    run("layout.set", { layout: root });
    if (!dropping && this.keep && list.includes(this.keep)) run("pane.focus", { terminal: this.keep });
  }

  // Full screen for the terminal, or the grid again when the space has a full screen tile.
  zoom(terminal) {
    const space = spaces().find((s) => s.active);
    if (!space || !terminal) return;
    const map = { ...(shared("helm.zoom") ?? {}) };
    if (map[space.id]) delete map[space.id];
    else map[space.id] = terminal;
    setShared("helm.zoom", map);
  }

  // The keyboard to another tile of the shown space: `by` from the focused one, or tile `index`.
  // The grid's order: the main tile, then the right half row by row.
  focusTile(by, index = null) {
    const space = spaces().find((s) => s.active);
    const zoomed = (shared("helm.zoom") ?? {})[space?.id];
    const list = tilesOf(space, panes());
    if (!list.length) return;
    const i = list.indexOf(focused()?.terminal);
    const to = index !== null ? list[index] : list[(((Math.max(i, 0) + by) % list.length) + list.length) % list.length];
    if (!to) return;
    // In full screen the full screen tile changes to that one.
    if (zoomed && zoomed !== to) setShared("helm.zoom", { ...(shared("helm.zoom") ?? {}), [space.id]: to });
    run("pane.focus", { terminal: to });
  }

  closeFocused() {
    const terminal = focused()?.terminal;
    const tab = terminal && panes().find((p) => p.terminal === terminal)?.tab;
    if (tab) run("tab.close", { id: tab });
  }

  // The dialog of the last press (`session`) is just starting: a press now must not open a
  // second one. Once it has the keyboard, keys.json sends the presses to the dialog itself
  // ("mod:helm-spaces"). A shared write makes every mod render again, so the opening
  // writes nothing; the dialog writes "helm.switcher.closed" when it closes.
  starting() {
    const s = this.session;
    return Boolean(s) && shared("helm.switcher.closed") !== s.id && Date.now() - s.id < 1500;
  }

  // The overview stays open until you pick a space: a second press closes it.
  openSpaces(select) {
    if (this.starting() && this.session.mode === "overview") return;
    this.session = { id: Date.now(), offset: 0, mode: "overview" };
    this.openWindow({ mode: "overview", select, session: this.session.id });
  }

  // The spaces window: a popup without Herdr's frame, so that it does not dim the window and a
  // quick alt-tab shows nothing. Centered under the bar (the bar is as wide as the window).
  openWindow(p) {
    const { width: most, height } = OVERLAYS["helm-spaces"];
    const width = Math.min(most, Math.max(560, (this.width ?? 1200) - 80));
    overlay("helm-spaces", { ...p, height: height - 20 }, { frame: "none", width, at: { x: Math.max(0, ((this.width ?? width) - width) / 2), y: 54 } });
  }

  // alt-tab: the switcher. A press while it starts goes to it as a step ("offset": the sum of
  // the presses of that dialog), and the dialog catches up when it starts.
  switch(dir) {
    const s = this.session;
    if (this.starting() && s.mode === "switch") {
      s.offset += dir;
      return setShared("helm.switcher.step", { session: s.id, offset: s.offset, at: Date.now() });
    }
    this.session = { id: Date.now(), offset: 0, mode: "switch" };
    this.openWindow({ mode: "switch", dir, session: this.session.id });
  }

  readSystem(cx) {
    if (this.reading) return;
    this.reading = true;
    cx.spawn(async (cx) => {
      try {
        if (!can("system", "usage")) return (this.system = null);
        const { cpu, mem, total } = (await helm(["system"])).data ?? {};
        if ([cpu, mem, total].every((n) => Number.isFinite(n))) this.system = { cpu, mem, total };
      } catch {}
      finally { this.reading = false; cx.notify(); }
    });
  }

  // `herdr-helm info` of each project space, one after the other, into shared "helm.info"
  // (space -> info). Each is a process of its own: nothing waits for it.
  readInfo(cx) {
    if (this.readingInfo) return;
    this.readingInfo = true;
    cx.spawn(async (cx) => {
      try {
        const out = {};
        for (const x of shared("helm.spaces") ?? []) {
          if (!x.herdr_space) continue;
          const r = await helm(["info", "--space", x.name]);
          if (r.data) out[x.herdr_space] = r.data;
        }
        // A write makes every mod render again: only when something changed.
        const next = JSON.stringify(out);
        if (next !== this.info) { this.info = next; setShared("helm.info", out); }
      } finally { this.readingInfo = false; }
    });
  }

  // When each space last did something: an agent changed state, or you showed it.
  stamp(sp) {
    const now = Date.now();
    let dirty = false;
    const active = sp.find((s) => s.active)?.id;
    for (const s of sp) {
      const states = s.tabs.map((t) => t.status ?? "").join();
      let at = this.activity[s.id] ?? 0;
      for (const t of s.tabs) if (t.changed_at && t.changed_at > at) at = t.changed_at;
      if (s.id in this.seen && this.seen[s.id] !== states) at = now;
      if (s.id === active && this.active !== active) at = now;
      this.seen[s.id] = states;
      if (at !== (this.activity[s.id] ?? 0)) { this.activity[s.id] = at; dirty = true; }
    }
    this.active = active;
    const known = new Set(sp.map((s) => s.id));
    for (const id of Object.keys(this.activity)) if (!known.has(id)) { delete this.activity[id]; dirty = true; }
    if (dirty) setShared("helm.activity", { ...this.activity });
  }

  // The side panel is remembered per space.
  syncPanel(space, panel, body, enabled) {
    if (!space || !panel) return;
    const now = panel.hidden ? null : body?.active ?? null;
    if (space.id !== this.space) {
      const first = this.space === undefined;
      this.space = space.id; this.pending = null;
      if (!(first && !(space.id in this.panels))) {
        const stored = this.panels[space.id] ?? null, want = enabled.includes(stored) ? stored : null;
        if (want !== now) {
          this.pending = { want, until: Date.now() + 1000 };
          if (want) { run("layout.select", { stack: "panel", child: want }); run("layout.show", { id: "panel" }); }
          else run("layout.hide", { id: "panel" });
          return;
        }
      }
    }
    if (this.pending) {
      if (this.pending.want !== now && Date.now() < this.pending.until) return;
      this.pending = null;
    }
    if (this.panels[space.id] === now) return;
    this.panels[space.id] = now;
    localStorage.setItem("panels", JSON.stringify(this.panels));
  }

  icon(t, id, picture, action, { on = false, badge = null, badgeColor = t.accent, dot = null } = {}) {
    return h_flex().id(id).h(28).min_w(28).px(6).gap(4).flex_none().items_center().justify_center().rounded(7)
      .bg(on ? t.selected : t.chrome).text_color(on ? t.foreground : t.muted)
      .hover((s) => s.bg(t.hover).text_color(t.foreground))
      .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
      .on_click(action)
      .child(picture)
      .when(Boolean(badge), (el) => el.child(div().px(5).h(16).flex().items_center().rounded(8).text_size(10.5).font_semibold()
        .bg(badgeColor + "33").text_color(badgeColor).child(badge)))
      .when(Boolean(dot), (el) => el.child(div().size(6).rounded(3).flex_none().bg(dot)));
  }

  // The PRs of the space's branches and the org's open PRs in one popup. The space part is as
  // tall as its PRs (a few at most); both lists scroll in their part. A part that the code host
  // connector cannot serve does not show, and the popup is then as tall as the other part.
  openGithub() {
    const mine = can("code_host", "branch_prs"), org = can("code_host", "org_prs");
    if (!mine && !org) return;
    const space = spaces().find((s) => s.active);
    const prs = ((shared("github.cache") ?? {})[space?.id]?.repos ?? []).filter((r) => r.pr).length;
    const tall = Math.min(320, 46 + Math.max(1, prs) * 56);
    const top = { id: "github-space", type: "block", size: { px: tall }, content: { mod: "helm-github" } };
    const bottom = { id: "github-prs", type: "block", size: { weight: 3 }, content: { mod: "helm-prs" } };
    overlay("github", {}, {
      ...(org ? {} : { height: tall }),
      layout: { id: "github-popup", type: "column", divider: { width: 1, color: "border" },
        children: mine && org ? [top, bottom] : mine ? [{ ...top, size: { weight: 1 } }] : [bottom] },
    });
  }

  // The PRs of a space as one state, as the cards show it: Open (or Draft) with the worst CI
  // of the open ones, else Merged or Closed.
  prChip(t, prs) {
    if (!prs.length) return null;
    const live = new Map((shared("prs.data")?.sections ?? []).flatMap((s) => s.prs).map((p) => [p.key, p]));
    const open = prs.map((x) => live.get(x.key)).filter(Boolean);
    const states = prs.map((x) => (live.has(x.key) ? "OPEN" : x.state));
    const state = open.length ? (open.every((p) => p.isDraft) ? "Draft" : "Open")
      : states.every((s) => s === "MERGED") ? "Merged" : states.some((s) => s === "CLOSED" || s === "MERGED") ? "Closed" : null;
    if (!state) return null;
    const color = { Open: t.ansi2, Draft: t.dim, Merged: t.ansi5, Closed: t.ansi1 }[state];
    const cis = open.map((p) => p.ci);
    const worst = ["FAILURE", "ERROR", "PENDING", "EXPECTED", "SUCCESS"].find((c) => cis.includes(c));
    const ci = worst === "SUCCESS" ? t.ansi2 : worst === "FAILURE" || worst === "ERROR" ? t.ansi1 : worst ? t.ansi3 : null;
    const text = prs.length === 1 ? `${prs[0].key}` : `${prs.length} PRs`;
    return h_flex().flex_none().gap(5).items_center().text_size(11.5).text_color(t.dim)
      .child(text)
      .when(Boolean(ci), (el) => el.child(div().size(6).rounded(3).bg(ci)))
      .child(div().px(5).h(16).flex().items_center().rounded(4).text_size(10).bg(color + "26").text_color(color).child(state));
  }

  systemUse(t) {
    const s = this.system;
    if (!s) return null;
    const used = (s.total * s.mem / 100 / 1073741824).toFixed(1);
    const item = (label, pct, text) => h_flex().gap(4).items_center()
      .child(div().text_size(10).text_color(t.dim).child(label))
      .child(div().text_size(11).text_color(level(pct, t)).child(text));
    return h_flex().id("system").h(28).px(6).gap(8).flex_none().items_center().rounded(7).hover((st) => st.bg(t.hover))
      .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
      .when(can("system", "apps"), (el) => el.cursor_pointer().on_click(() => overlay("helm-usage-system", { system: true })))
      .child(item("cpu", s.cpu, `${Math.round(s.cpu)}%`))
      .child(item("mem", s.mem, `${used}/${Math.round(s.total / 1073741824)}G`));
  }

  render() {
    const t = theme();
    const sp = spaces();
    const space = sp.find((s) => s.active);
    const root = layout(), w = window();
    const lights = w?.platform === "macos" && w.titlebar === "none" && !w.fullscreen && root?.traffic_lights
      ? (root.traffic_lights[0] + 64) / settings().zoom : 10;
    // The panel is the stack itself: Herdr folds a node with one child into that child.
    const panel = find(root, "panel"), body = panel;
    const enabled = shared("workspace.panels") ?? ["files", "changes"];
    const panelIcon = (id, picture, badge) => {
      const on = panel && !panel.hidden && body?.active === id;
      return this.icon(t, `panel-${id}`, svg(picture).size(15), () => {
        if (on) run("layout.hide", { id: "panel" });
        else { run("layout.select", { stack: "panel", child: id }); run("layout.show", { id: "panel" }); }
      }, { on, badge });
    };
    const task = (shared("helm.spaces") ?? []).find((x) => x.herdr_space === space?.id);
    const mine = agents().filter((a) => a.space === space?.id);
    const busy = mine.filter((a) => a.status === "working").length;
    const counts = mine.length ? `${busy ? `${busy} busy · ` : ""}${mine.length} ${mine.length === 1 ? "agent" : "agents"}` : null;
    const waiting = sp.filter((s) => !s.active && (s.status === "blocked" || s.status === "done"));
    const blocked = waiting.some((s) => s.status === "blocked");
    const count = shared("changes.count");
    const changes = count && count.space === space?.id ? count : null;
    const changesBadge = !changes ? null : changes.files ? String(changes.files) : changes.ahead ? `↑${changes.ahead}` : null;
    const prs = shared("prs.data");
    const needs = ["attention", "review"].reduce((n, key) => n + (prs?.sections.find((s) => s.key === key)?.prs.length ?? 0), 0);
    const github = space ? (shared("github.cache") ?? {})[space.id] : null;
    const checks = (github?.repos ?? []).filter((r) => r.pr?.state === "OPEN").map((r) => r.pr.checks);
    const worst = ["failing", "pending", "passing"].find((c) => checks.includes(c));
    const checkColor = { failing: t.ansi1, pending: t.ansi3, passing: t.ansi2 }[worst] ?? null;
    const usage = (shared("agent.usage")?.limits ?? []).filter((l) => l.kind !== "weekly_scoped");
    const top = usage.reduce((m, l) => Math.max(m, l.percent), 0);
    const usageColor = top >= 90 ? t.ansi1 : top >= 60 ? t.ansi3 : t.dim;
    const pr = this.prChip(t, task?.prs ?? []);
    const system = this.systemUse(t);
    return h_flex().size_full().min_w(0).pl(lights).pr(8).gap(4).items_center().bg(t.chrome).border_b(1).border_color(t.border)
      .on_size_change((size) => { this.width = size.width; })
      // A drag released here ends without a swap.
      .on_mouse_up("left", () => { if (shared("helm.drag")) setShared("helm.drag", null); })
      .child(this.icon(t, "overview", svg("mods/helm-bar/grid.svg").size(15), () => this.openSpaces(null)))
      // The actions of the space, as buttons.
      .when(Boolean(space), (el) => el.child(actionButtons(t, space.id)))
      .child(h_flex().flex_1().min_w(0).px(8).gap(10).items_center()
        .child(div().flex_shrink(1).min_w(0).truncate().text_size(13).font_semibold().text_color(t.foreground).child(space?.name ?? ""))
        .when(Boolean(task?.project), (el) => el.child(div().flex_none().text_size(11.5).text_color(t.dim).child(task.project)))
        .when(Boolean(pr), (el) => el.child(pr))
        .when(Boolean(counts), (el) => el.child(h_flex().flex_none().gap(5).items_center().text_size(11.5).text_color(t.dim)
          .when(busy > 0, (row) => row.child(svg("mods/workspace-chrome/spinner.svg").size(11).text_color(t.working).spin(900)))
          .child(counts)))
        .when(waiting.length > 0, (el) => el.child(h_flex().id("waiting").flex_none().h(22).px(8).gap(6).items_center().rounded(6)
          .bg((blocked ? t.blocked : t.done) + "22").text_size(11.5).font_semibold().text_color(blocked ? t.blocked : t.done)
          .hover((st) => st.bg((blocked ? t.blocked : t.done) + "33"))
          .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
          .on_click(() => this.openSpaces("waiting"))
          .child(div().size(6).rounded(3).bg(blocked ? t.blocked : t.done))
          .child(`${waiting.length} ${waiting.length === 1 ? "space needs" : "spaces need"} you`))))
      .when(Boolean(system), (el) => el.child(system))
      .when(can("agent", "usage"), (el) => el.child(this.icon(t, "usage", svg(agentIcon(t).path).size(14).text_color(agentIcon(t).color),
        () => this.openUsage(),
        { badge: usage.length ? `${Math.round(top)}%` : null, badgeColor: usageColor })))
      .when(can("code_host", "branch_prs") || can("code_host", "org_prs"), (el) => el.child(this.icon(t, "github", svg("mods/helm-bar/github.svg").size(15), () => this.openGithub(),
        { badge: needs ? String(needs) : null, badgeColor: t.ansi1, dot: checkColor })))
      .when(enabled.includes("changes"), (el) => el.child(panelIcon("changes", "mods/helm-changes/changes.svg", changesBadge)))
      .when(enabled.includes("files"), (el) => el.child(panelIcon("files", "mods/helm-files/file.svg", null)))
      // The actions of the layout with their keys: Herdr gives mods no macOS menu bar.
      // A dot when a newer Helm is there: the ⋯ menu has the Update row.
      .child(this.icon(t, "menu", svg("mods/helm-bar/more.svg").size(15),
        () => this.command("menu"), { dot: shared("helm.update")?.available ? t.accent : null }));
  }
}
