// The header of a tile in the helm grid, props { terminal, at } (at: its number, as cmd-1 to
// cmd-9 use it), or { empty: true } for a space without terminals, or { drop: terminal } for
// the drop area that takes the place of a terminal while a tile is dragged. Shows the program's icon, what the agent calls its work (else the tab's
// name, else the program), the folder, the status, and the buttons: Make main (a tile on the
// right half; props().main is false), restart (an agent tile: `herdr-helm restart` stops the
// agent and starts it again in the same session, for example after an update), full screen, and
// close (ends the terminal, as the close button of Herdr's pane header does).
// Click: the keyboard goes to the tile's terminal.
// Drag a title bar onto another tile to swap the two. Each header is its own mod, so the drag
// goes through shared "helm.drag" ({ terminal, space, at, active }). Once the pointer leaves
// the title bar, the drag is active and the tiler shows the drop areas. The drop asks the tiler
// for the swap ("helm.move"). A release on the bar or on its own tile ends the drag; else the
// bar ends it after 10 seconds.

import { View, div, svg } from "gpui-kit";
import { h_flex, v_flex } from "gpui-base";
import { panes, spaces, focused, props, run, theme, shared, setShared } from "herdr";
import { overlay, can, isAgent, restartAgent } from "./helm.js";
import { programIcon, shortHome } from "./ui.js";

// The agent's own title, when it says more than the program's name or a folder.
function agentTitle(p) {
  const live = (p?.title ?? "").replace(/^[^\p{L}\p{N}]+/u, "").trim();
  if (!live || live === p.program?.name || live.toLowerCase() === p.program?.key) return null;
  return /^[~/]|^[A-Za-z]:\\/.test(live) || live.includes("@") ? null : live;
}

export default class Tile extends View {
  init() {
    this.hovered = false;
    this.restarting = false;
    this.armed = 0; // a restart of a working agent needs a second click within 3 s
  }

  // The agent again in the same session. When it started as the terminal's own command, a new
  // terminal takes its place in the tile order.
  restart(terminal, space, status, cx) {
    if (this.restarting) return;
    if (status === "working" && Date.now() - this.armed > 3000) {
      this.armed = Date.now();
      cx.notify();
      cx.spawn(async (cx) => { await cx.sleep(3100); cx.notify(); });
      return;
    }
    this.armed = 0;
    this.restarting = true;
    cx.notify();
    cx.spawn(async (cx) => {
      try { await restartAgent(terminal, space); }
      finally { this.restarting = false; cx.notify(); }
    });
  }

  button(t, id, picture, action) {
    return div().id(id).p(3).rounded(4).flex_none().text_color(t.dim)
      .hover((s) => s.bg(t.hover).text_color(t.foreground))
      // A press on a button is no drag of the title bar.
      .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
      .on_click((_e, cx) => { cx.stop_propagation(); action(cx); })
      .child(svg(picture).size(14));
  }

  // Ends the drag of `d` on tile `terminal`: a swap when it is another tile of the space.
  dropOn(terminal, space) {
    const d = shared("helm.drag");
    if (!d) return;
    if (d.terminal !== terminal && d.space === space) setShared("helm.move", { space, a: d.terminal, b: terminal, at: Date.now() });
    setShared("helm.drag", null);
  }

  // Where a dragged tile can land: the whole tile under the header.
  dropArea(t) {
    const terminal = props().drop;
    const p = panes().find((x) => x.terminal === terminal);
    const space = spaces().find((s) => s.tabs.some((x) => x.id === p?.tab));
    const d = shared("helm.drag");
    const own = d?.terminal === terminal;
    return v_flex().id(`drop-${terminal}`).size_full().items_center().justify_center().gap(6).rounded_b(10)
      .bg(this.hovered && !own ? t.accent + "1f" : t.background)
      .when(this.hovered && !own, (el) => el.border_2().border_color(t.accent))
      .on_hover((hovered, cx) => { this.hovered = hovered; cx.notify(); })
      .on_mouse_up("left", () => this.dropOn(terminal, space?.id))
      .child(div().text_size(22).font_semibold().text_color(own ? t.dim : t.muted).child(`[${props().at}]`))
      .child(div().text_size(12).text_color(t.dim).child(own ? "Release here to cancel" : "Drop to swap"));
  }

  empty(t) {
    return v_flex().size_full().items_center().justify_center().gap(10).text_color(t.dim).text_size(13)
      .child("No terminals in this space.")
      .child(h_flex().id("empty-new").px(10).h(28).gap(6).items_center().rounded(7).border(1).border_color(t.border)
        .text_color(t.muted).hover((s) => s.bg(t.hover).text_color(t.foreground))
        .on_click(() => overlay("new-menu"))
        .child(svg("mods/workspace-chrome/plus.svg").size(13)).child("Start a terminal or an agent"));
  }

  render() {
    const t = theme();
    if (props()?.empty) return this.empty(t);
    if (props()?.drop) return this.dropArea(t);
    const terminal = props()?.terminal;
    const p = panes().find((x) => x.terminal === terminal);
    if (!p) return div();
    const space = spaces().find((s) => s.tabs.some((x) => x.id === p.tab));
    const tab = space?.tabs.find((x) => x.id === p.tab);
    // A name someone gave the tab wins.
    const name = tab?.named ? tab.name : agentTitle(p) ?? p.program?.name ?? "Terminal";
    const zoomed = space && (shared("helm.zoom") ?? {})[space.id] === terminal;
    const zoom = () => {
      const map = { ...(shared("helm.zoom") ?? {}) };
      if (zoomed) delete map[space.id];
      else map[space.id] = terminal;
      setShared("helm.zoom", map);
    };
    // To place 1: the main tile.
    const makeMain = () => setShared("helm.move", { space: space.id, a: terminal, b: null, at: Date.now() });
    const drag = shared("helm.drag");
    const dragged = drag?.terminal === terminal;
    const target = Boolean(drag) && !dragged && drag.space === space?.id && this.hovered;
    const drop = () => this.dropOn(terminal, space?.id);
    // The layout's focus border does not see a terminal block: the header shows the focus.
    const on = focused()?.terminal === terminal;
    const pic = programIcon(p.program, 14, on ? t.foreground : t.dim);
    const st = p.status;
    const agent = isAgent(p) && can("agent", "restart");
    const armed = Date.now() - this.armed < 3000;
    return h_flex().id(`tile-head-${terminal}`).size_full().px(10).gap(8).items_center()
      .bg(t.background).border_b(on ? 2 : 1).border_color(on ? t.accent : t.border).text_size(12)
      .when(target, (el) => el.border_2().border_color(t.accent))
      .when(dragged, (el) => el.opacity(0.5))
      .on_hover((hovered, cx) => {
        this.hovered = hovered;
        // Pressed here and now out of the title bar: the drag starts, and the drop areas show.
        const d = shared("helm.drag");
        if (!hovered && d?.terminal === terminal && !d.active) setShared("helm.drag", { ...d, active: true });
        cx.notify();
      })
      .on_mouse_down("left", () => { if (space) setShared("helm.drag", { terminal, space: space.id, at: Date.now() }); })
      .on_mouse_up("left", () => drop())
      .on_click(() => run("pane.focus", { terminal }))
      .when(Boolean(props()?.at), (el) => el.child(div().flex_none().text_size(11).font_semibold().text_color(on ? t.accent : t.dim).child(`[${props().at}]`)))
      .when(Boolean(pic), (el) => el.child(pic))
      .child(div().flex_shrink(1).min_w(0).truncate().font_medium().text_color(on ? t.foreground : t.muted).child(name))
      .child(div().flex_1().min_w(0).truncate().text_color(t.dim).child(shortHome(p.cwd)))
      .when(st === "working", (el) => el.child(svg("mods/workspace-chrome/spinner.svg").size(12).flex_none().text_color(t.working).spin(900)))
      .when(st === "blocked", (el) => el.child(div().flex_none().px(6).h(18).flex().items_center().rounded(5).bg(t.blocked + "26")
        .text_size(10.5).font_semibold().text_color(t.blocked).child("Needs you")))
      .when(st === "done", (el) => el.child(div().size(7).flex_none().rounded(4).bg(t.done)))
      .when(Boolean(space) && props()?.main === false && !zoomed, (el) => el.child(this.button(t, `tile-main-${terminal}`, "mods/helm-tile/main.svg", makeMain)))
      .when(agent && Boolean(space), (el) => el.child(this.restarting
        ? div().p(3).flex_none().child(svg("mods/workspace-chrome/spinner.svg").size(14).text_color(t.dim).spin(900))
        : armed
          ? div().id(`tile-restart-${terminal}`).px(6).h(20).flex_none().flex().items_center().rounded(4).text_size(10.5).font_semibold()
            .bg(t.blocked + "26").text_color(t.blocked)
            .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
            .on_click((_e, cx) => { cx.stop_propagation(); this.restart(terminal, space.id, st, cx); })
            .child("Busy: restart?")
          : this.button(t, `tile-restart-${terminal}`, "mods/helm-tile/restart.svg", (cx) => this.restart(terminal, space.id, st, cx))))
      .when(Boolean(space), (el) => el.child(this.button(t, `tile-zoom-${terminal}`, `mods/pane/${zoomed ? "unzoom" : "zoom"}.svg`, zoom)))
      .child(this.button(t, `tile-close-${terminal}`, "mods/pane/close.svg", () => run("pane.close", { pane: p.pane })));
  }
}
