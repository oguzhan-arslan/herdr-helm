// The top part of the GitHub popup of helm: the PRs of the shown space's branches, and
// which tiles work on each branch. The data is what helm-jobs keeps in shared "github.cache";
// the tile numbers come from the bar ("helm.tiles"). A tile
// works on a branch when its folder is in the checkout and it is on that branch.
// Click a PR to open it, a tile number to give that tile the keyboard.

import { View, div, svg } from "gpui-kit";
import { v_flex, h_flex, Scrollbar } from "gpui-base";
import { spaces, panes, run, theme, shared, setShared, props, closeOverlay } from "herdr";
import { agoText } from "./ui.js";

const STATE_COLOR = { Open: "#3fae6a", Draft: "#8b949e", Merged: "#8b7cf6", Closed: "#d9649a" };
const REVIEW = { APPROVED: "approved", CHANGES_REQUESTED: "changes requested", REVIEW_REQUIRED: "review required" };
const CHECK = { passing: "✓", failing: "✗", pending: "●" };

export default class HelmGithub extends View {
  // The tiles on the repo's branch, by number: "[2]".
  tilesOn(r, tiles, ps) {
    return tiles.list.map((terminal, i) => ({ terminal, at: i + 1, p: ps.find((x) => x.terminal === terminal) }))
      .filter(({ p }) => p?.cwd && (p.cwd === r.path || p.cwd.startsWith(`${r.path}/`)) && p.branch === r.branch);
  }

  row(r, tiles, ps, t) {
    const pr = r.pr;
    const state = pr.isDraft && pr.state === "OPEN" ? "Draft" : pr.state[0] + pr.state.slice(1).toLowerCase();
    const color = STATE_COLOR[state] ?? t.dim;
    const checks = { passing: t.ansi2, pending: t.ansi3, failing: t.ansi1 };
    const on = this.tilesOn(r, tiles, ps);
    const sync = [r.ahead ? `↑${r.ahead}` : null, r.behind ? `↓${r.behind}` : null, r.dirty ? `${r.dirty} changed` : null].filter(Boolean).join(" ");
    return v_flex().id(`pr-${r.repo}`).flex_none().px(8).py(7).gap(4).rounded(7).cursor_pointer().hover((s) => s.bg(t.hover))
      .on_click((_e, cx) => cx.open_url(pr.url))
      .child(h_flex().gap(8).items_center()
        .child(div().flex_none().px(5).h(16).flex().items_center().rounded(4).text_size(10).font_semibold().bg(color + "26").text_color(color).child(state))
        .child(div().flex_1().min_w(0).truncate().text_color(t.foreground).child(`#${pr.number} ${pr.title}`))
        .when(Boolean(pr.checks), (el) => el.child(div().flex_none().text_color(checks[pr.checks] ?? t.dim).child(CHECK[pr.checks] ?? ""))))
      .child(h_flex().gap(6).items_center().text_size(11).text_color(t.dim)
        .child(div().flex_shrink(1).min_w(0).truncate().child(`${r.repo} · ${r.branch}`))
        .when(Boolean(sync), (el) => el.child(div().flex_none().text_color(t.muted).child(sync)))
        .when(pr.state === "OPEN" && Boolean(REVIEW[pr.reviewDecision]), (el) => el.child(div().flex_none().child(`· ${REVIEW[pr.reviewDecision]}`)))
        .child(div().flex_1())
        .children(on.map(({ terminal, at }) => div().id(`pr-${r.repo}-tile-${at}`).flex_none().px(5).h(16).flex().items_center().rounded(4)
          .text_size(10.5).font_semibold().bg(t.accent + "22").text_color(t.accent).hover((s) => s.bg(t.accent + "44"))
          .on_click((_e, cx) => { cx.stop_propagation(); run("pane.focus", { terminal }); closeOverlay(props().overlay); })
          .child(`[${at}]`))));
  }

  render() {
    const t = theme();
    const space = spaces().find((s) => s.active);
    const mine = space ? (shared("github.cache") ?? {})[space.id] : null;
    const tiles = shared("helm.tiles")?.space === space?.id ? shared("helm.tiles") : { list: [] };
    const ps = panes();
    const withPr = (mine?.repos ?? []).filter((r) => r.pr).sort((a, b) => (a.pr.state === "OPEN" ? 0 : 1) - (b.pr.state === "OPEN" ? 0 : 1));
    const note = (text) => div().px(8).py(6).text_size(11.5).text_color(t.dim).child(text);
    return v_flex().id("helm-github").size_full().p(5).bg(t.popover).text_size(12.5)
      .child(h_flex().px(8).pt(4).pb(4).gap(8).items_center()
        .child(svg("mods/helm-bar/github.svg").size(13).text_color(t.muted))
        .child(div().flex_1().truncate().text_size(11).text_color(t.dim)
          .child(`PRs of ${space?.name ?? "this space"}${mine ? ` · updated ${agoText(mine.at)}` : ""}`))
        .child(div().id("helm-github-refresh").px(6).rounded(5).cursor_pointer().text_size(11).text_color(t.muted).hover((s) => s.bg(t.hover))
          .on_click(() => setShared("github.refresh", Date.now())).child("Refresh")))
      .when(Boolean(mine?.error), (el) => el.child(note(mine.error)))
      .when(!mine, (el) => el.child(note("Reading the repos…")))
      .when(Boolean(mine) && !mine.error && withPr.length === 0, (el) => el.child(note("No PRs from the branches of this space.")))
      .child(div().relative().flex_1().min_h(0)
        .child(v_flex().id("helm-github-list").size_full().overflow_y_scroll().children(withPr.map((r) => this.row(r, tiles, ps, t))))
        .child(Scrollbar.vertical("helm-github-list").absolute().inset_0()));
  }
}
