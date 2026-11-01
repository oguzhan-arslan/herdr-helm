// The spaces window of the helm layout: the spaces per project on the left, all that is
// known of the highlighted one on the right. Arrows and tab move, enter shows the space,
// typing filters, esc closes. It is a popup without Herdr's frame (a dialog dims the window)
// and draws its own.
//   props { mode: "overview", select }: alt-space or the grid button. It shows at once and
//     stays open. select "waiting": the first space that needs you.
//   props { mode: "switch", dir }: alt-tab, with the last space before the shown one picked.
//     A quick alt-tab (alt released within QUICK ms, no further tab) shows that space and
//     closes before the window ever shows. A longer hold or a further tab shows the window,
//     and it then stays open like the overview. A further press while it starts comes from the
//     bar as shared "helm.switcher.step".
// While it has the keyboard, keys.json sends alt-tab, alt-shift-tab and alt-space here
// ("mod:helm-spaces"). It writes no shared value until it closes: each write makes every
// mod render again, and the opening would wait for them.
// It reads what helm-jobs keeps in shared ("helm.projects", "helm.spaces", "prs.data"). The details are `herdr-helm info` of each space,
// which the bar reads in the background ("helm.info"). Herdr stops a render that takes too
// long: keep the work per render small.

import { View, div, svg } from "gpui-kit";
import { v_flex, h_flex, Scrollbar } from "gpui-base";
import { actionButtons, lookAtCheckout } from "./space-actions.js";
import { overlay } from "./helm.js";
import { ago, programIcon, shortHome } from "./ui.js";
import { spaces, agents, run, closeOverlay, props, theme, shared, setShared, nextCommand } from "herdr";

const clean = (s) => (s ?? "").replace(/^[^\p{L}\p{N}]+/u, "").trim();

function mark(t, s, small = false) {
  if (s === "working") return svg("mods/workspace-chrome/spinner.svg").size(12).flex_none().text_color(t.working).spin(900);
  if (s === "blocked" && !small) return div().flex_none().px(6).h(18).rounded(5).bg(t.blocked + "26").text_size(10.5).font_semibold().text_color(t.blocked).flex().items_center().child("Needs you");
  if (s === "blocked" || s === "done") return div().size(7).mx(3).flex_none().rounded(4).bg(t[s]);
  return div().size(7).mx(3).flex_none();
}

const QUICK = 300; // ms: an alt-tab released within this goes back to the last space
const HIDDEN = 150; // ms: how long a switch stays invisible, so a quick one does not flash

export default class Spaces extends View {
  init(_props, cx) {
    this.mode = props()?.mode ?? "overview";
    // Stays open after alt is released: the overview always, a switch once held or moved.
    this.sticky = this.mode !== "switch";
    this.visible = this.mode !== "switch";
    this.released = false;
    if (this.mode === "switch") {
      cx.timer.after(HIDDEN, (cx) => { if (!this.closed) { this.visible = true; cx.notify(); } });
      cx.timer.after(QUICK, () => { if (!this.released) this.sticky = true; });
    }
    this.filter = "";
    this.session = props()?.session ?? null;
    this.applied = 0; // the presses of this session that moved the selection
    this.handle = cx.focus_handle();
    this.handle.focus();
    // alt-tab and alt-space while this has the keyboard (keys.json "mod:helm-spaces").
    cx.spawn(async (cx) => {
      for (;;) {
        const c = await nextCommand();
        if (c === "next" || c === "prev") { this.move(c === "next" ? 1 : -1); this.held(); cx.notify(); }
        if (c === "close") this.dismiss();
      }
    });
    const list = this.list();
    const active = spaces().find((s) => s.active)?.id;
    if (this.mode === "switch") {
      // The last space before the shown one: the one a quick alt-tab goes back to.
      const activity = shared("helm.activity") ?? {};
      const last = spaces().filter((s) => !s.active).sort((a, b) => (activity[b.id] ?? 0) - (activity[a.id] ?? 0))[0];
      this.sel = last?.id ?? active;
    } else {
      const waiting = props()?.select === "waiting" && list.find((s) => !s.active && (s.status === "blocked" || s.status === "done"));
      this.sel = waiting ? waiting.id : active;
    }
    // The bar's presses of this session, also those from before this dialog started. It is fast:
    // each alt-tab must move the selection at once.
    cx.timer.every(50, (cx) => {
      if (this.sel) lookAtCheckout(this.sel, cx);
      const step = shared("helm.switcher.step");
      if (step?.session !== this.session) return;
      if (step.close) return this.dismiss();
      let delta = step.offset - this.applied;
      if (!delta) return;
      this.applied = step.offset;
      this.move(delta);
      this.held();
      cx.notify();
    });
  }

  // A further tab or a move: the window shows and stays open.
  held() {
    this.sticky = true;
    this.visible = true;
  }

  // alt released: a quick alt-tab goes back to the last space; otherwise the window stays.
  released_(e) {
    if (this.mode !== "switch" || this.released || e.modifiers.alt) return;
    this.released = true;
    if (!this.sticky) this.show();
  }

  dismiss() {
    if (this.closed) return;
    this.ended();
    closeOverlay(props().overlay);
  }

  // Herdr's hook, also when it closes from outside (a click beside it): the bar must open a
  // new one next time.
  close() {
    this.ended();
  }

  ended() {
    if (this.closed && this.endedOnce) return;
    this.closed = this.endedOnce = true;
    setShared("helm.switcher.closed", this.session);
  }

  // Per project, by last activity in each; typing filters by name and project.
  list() {
    const sp = spaces();
    const activity = shared("helm.activity") ?? {};
    const byActivity = (a, b) => (activity[b.id] ?? 0) - (activity[a.id] ?? 0);
    const tasks = shared("helm.spaces") ?? [];
    const projects = (shared("helm.projects") ?? []).map((p) => p.name);
    const projectOf = (s) => {
      const name = tasks.find((x) => x.herdr_space === s.id)?.project;
      return projects.includes(name) ? name : null;
    };
    const f = this.filter.toLowerCase();
    const hit = (s) => !f || s.name.toLowerCase().includes(f) || (projectOf(s) ?? "other").toLowerCase().includes(f);
    return [...projects, null].flatMap((p) => sp.filter((s) => projectOf(s) === p && hit(s)).sort(byActivity).map((s) => Object.assign(s, { project: p })));
  }

  move(by) {
    const list = this.list();
    if (!list.length || !by) return;
    const i = Math.max(0, list.findIndex((s) => s.id === this.sel));
    this.sel = list[(((i + by) % list.length) + list.length) % list.length].id;
  }

  show(id = this.sel, terminal = null) {
    if (id) run("space.select", { id });
    if (terminal) run("pane.focus", { terminal });
    this.dismiss();
  }

  key(e, cx) {
    const k = e.keystroke;
    if (k === "escape") return this.dismiss();
    // Space shows the space too, while the filter is empty; after text it is part of the filter.
    if (k === "enter" || (k === "space" && !this.filter)) return this.show();
    // With alt still held (alt-tab), the arrows come as alt-down and alt-up.
    if (["down", "alt-down", "tab", "right"].includes(k)) { this.move(1); this.held(); }
    else if (["up", "alt-up", "shift-tab", "left"].includes(k)) { this.move(-1); this.held(); }
    else if (k === "backspace") { this.filter = this.filter.slice(0, -1); this.keepSelection(); }
    else if (k === "cmd-backspace") { this.filter = ""; this.keepSelection(); }
    else if (k === "alt-backspace") { this.filter = this.filter.replace(/\S*\s*$/, ""); this.keepSelection(); }
    else if (e.key_char && !e.modifiers.platform && !e.modifiers.control && !e.modifiers.alt) {
      this.filter += e.key_char;
      this.keepSelection();
    } else return;
    cx.stop_propagation();
    cx.notify();
  }

  keepSelection() {
    const list = this.list();
    if (!list.some((s) => s.id === this.sel)) this.sel = list[0]?.id ?? null;
  }

  row(s, t) {
    const on = s.id === this.sel;
    return h_flex().id(`row-${s.id}`).h(30).px(10).gap(8).flex_none().items_center().rounded(7)
      .bg(on ? t.selected : t.popover).hover((st) => st.bg(on ? t.selected : t.hover))
      .on_mouse_move((_e, cx) => { if (this.sel !== s.id) { this.sel = s.id; this.held(); cx.notify(); } })
      .on_click(() => this.show(s.id))
      .child(div().w(14).flex_none().flex().justify_center().child(mark(t, s.status, true)))
      .child(div().flex_1().min_w(0).truncate().text_color(on || s.active ? t.foreground : t.muted).when(s.active, (el) => el.font_semibold()).child(s.name));
  }

  left(t) {
    const list = this.list();
    const rows = [];
    let group;
    for (const s of list) {
      if (s.project !== group) {
        group = s.project;
        const project = group;
        rows.push(h_flex().id(`group-${group ?? "other"}`).h(28).pl(10).pr(4).pt(6).gap(6).flex_none().items_center().text_size(11).text_color(t.dim)
          .child(svg(`mods/workspace-chrome/${group ? "folder" : "layers"}.svg`).size(12))
          .child(div().flex_1().child(group ?? "Other"))
          // A new space there: the New space dialog with this project (or Other) picked.
          .child(div().id(`group-new-${group ?? "other"}`).size(22).flex_none().flex().items_center().justify_center().rounded(5)
            .hover((st) => st.bg(t.hover).text_color(t.foreground))
            .on_click(() => {
              this.dismiss();
              overlay("helm-task", { mode: "new", ...(project ? { project } : { other: true }) });
            })
            .child(svg("mods/workspace-chrome/plus.svg").size(12))));
      }
      rows.push(this.row(s, t));
    }
    if (!rows.length) rows.push(div().p(10).text_color(t.dim).child("No space matches."));
    return v_flex().w(320).flex_none().h_full().border_r(1).border_color(t.border)
      .child(h_flex().h(36).flex_none().px(12).gap(8).items_center().border_b(1).border_color(t.border)
        .child(svg("mods/helm-bar/search.svg").size(13).text_color(t.dim))
        .child(div().flex_1().truncate().text_color(this.filter ? t.foreground : t.dim).child(this.filter || "Type to filter")))
      .child(div().relative().flex_1().min_h(0)
        .child(v_flex().id("space-list").size_full().p(6).gap(1).overflow_y_scroll().children(rows))
        .child(Scrollbar.vertical("space-list").absolute().inset_0()));
  }

  section(t, title, children) {
    return v_flex().gap(3).pt(12)
      .child(div().text_size(10.5).font_semibold().text_color(t.dim).child(title))
      .children(children);
  }

  line(t, id, main, sub, act) {
    return v_flex().id(id).flex_none().py(4).gap(2).rounded(6)
      .when(Boolean(act), (el) => el.px(6).mx(-6).hover((s) => s.bg(t.hover)).on_click((_e, cx) => { cx.stop_propagation(); act(cx); }))
      .child(main)
      .when(Boolean(sub), (el) => el.child(div().text_size(11).text_color(t.dim).child(sub)));
  }

  chip(text, color) {
    return div().flex_none().px(5).h(16).flex().items_center().rounded(4).text_size(10).font_semibold().bg(color + "26").text_color(color).child(text);
  }

  details(t) {
    const s = spaces().find((x) => x.id === this.sel);
    if (!s) return div().flex_1();
    const task = (shared("helm.spaces") ?? []).find((x) => x.herdr_space === s.id);
    // The bar reads these in the background ("helm.info"): opening and moving here run nothing.
    const d = (shared("helm.info") ?? {})[s.id];
    const tabs = new Map(s.tabs.map((x) => [x.id, x]));
    const mine = agents().filter((a) => a.space === s.id);
    const live = new Map((shared("prs.data")?.sections ?? []).flatMap((x) => x.prs).map((p) => [p.key, p]));
    const stateColor = { OPEN: t.ansi2, DRAFT: t.dim, MERGED: t.ansi5, CLOSED: t.ansi1 };
    const workColor = { active: t.ansi2, waiting: t.ansi3, done: t.dim };
    const agentRows = mine.map((a) => {
      const tab = tabs.get(a.tab);
      const name = tab?.named ? tab.name : clean(a.title) || a.program?.name || "Agent";
      const pic = programIcon(a.program, 13, t.foreground);
      return this.line(t, `ag-${a.address}`, h_flex().gap(8).items_center()
        .when(Boolean(pic), (el) => el.child(pic))
        .child(div().flex_1().min_w(0).truncate().text_color(t.foreground).child(name))
        .child(mark(t, a.status)), null, () => this.show(s.id, a.address));
    });
    const prs = (d?.prs ?? task?.prs ?? []).map((x) => {
      const p = live.get(x.key);
      const state = p ? (p.isDraft ? "DRAFT" : "OPEN") : x.state ?? "?";
      const why = p?.reasons?.length ? p.reasons.join(", ") : x.author ? `by ${x.author}` : "";
      return this.line(t, `pr-${x.key}`, h_flex().gap(8).items_center()
        .child(this.chip(state[0] + state.slice(1).toLowerCase(), stateColor[state] ?? t.dim))
        .child(div().flex_1().min_w(0).truncate().text_color(t.foreground).child(`${x.repo}#${x.number} ${x.title ?? ""}`)),
        why, x.url ? (cx) => cx.open_url(x.url) : null);
    });
    const work = (d?.work ?? []).map((w, i) => this.line(t, `work-${i}`, h_flex().gap(8).items_center()
      .child(this.chip(w.state, workColor[w.state] ?? t.dim))
      .child(div().flex_1().min_w(0).truncate().text_color(t.foreground).child(w.title))));
    const checkouts = (d?.checkouts ?? []).map((k) => this.line(t, `co-${k.folder}`, h_flex().gap(8).items_center()
      .child(div().flex_none().font_semibold().text_color(t.foreground).child(k.folder))
      .child(div().flex_1().min_w(0).truncate().text_size(11.5).text_color(t.muted).child(k.branch)),
      k.missing ? "The folder is missing." : [`${k.commits} commits`, k.dirty && `${k.dirty} changed`, k.unpushed && `${k.unpushed} not pushed`,
        k.behind && `${k.behind} behind`].filter(Boolean).join(" · ")));
    const sessions = (d?.sessions ?? []).slice(0, 6).map((x) => this.line(t, `s-${x.session}`, h_flex().gap(8).items_center()
      .child(div().flex_1().min_w(0).truncate().text_color(t.muted).child(x.title || x.prompt || x.session))
      .child(div().flex_none().text_size(11).text_color(t.dim).child(ago(x.at)))));
    const sub = [task?.project ?? (task ? null : "Other"), s.branch, d ? `named by ${d.title_source}` : null].filter(Boolean).join(" · ");
    return div().relative().flex_1().min_w(0).h_full()
      .child(v_flex().id("details").size_full().px(18).py(14).overflow_y_scroll().text_size(12.5)
        .child(h_flex().gap(8).items_center()
          .child(div().flex_1().min_w(0).text_size(16).font_semibold().text_color(t.foreground).child(s.name))
          // The space's actions; the dialog closes before each one.
          .child(actionButtons(t, s.id, () => this.dismiss())))
        .child(div().pt(2).text_size(11.5).text_color(t.dim).child(sub))
        .when(Boolean(d?.summary), (el) => el.child(this.section(t, "NOW", [div().text_color(t.muted).child(d.summary)])))
        .when(work.length > 0, (el) => el.child(this.section(t, "WORK", work)))
        .child(this.section(t, "AGENTS", agentRows.length ? agentRows : [div().text_size(11.5).text_color(t.dim).child("No agents.")]))
        .when(prs.length > 0, (el) => el.child(this.section(t, "PULL REQUESTS", prs)))
        .when(checkouts.length > 0, (el) => el.child(this.section(t, "CHECKOUTS", checkouts)))
        .when(sessions.length > 0, (el) => el.child(this.section(t, `${(shared("helm.connectors")?.agent?.label ?? "Agent").toUpperCase()} SESSIONS`, sessions)))
        .when(Boolean(task) && !d, (el) => el.child(div().pt(12).text_size(11.5).text_color(t.dim).child("Reading the space…")))
        .child(this.section(t, "FOLDER", [div().truncate().text_color(t.muted).child(shortHome(d?.path ?? s.folder))])))
      .child(Scrollbar.vertical("details").absolute().inset_0());
  }

  render() {
    const t = theme();
    const button = (id, picture, label, action) => h_flex().id(id).h(26).px(9).gap(6).flex_none().items_center().rounded(7)
      .border(1).border_color(t.border).text_size(11.5).text_color(t.muted).hover((s) => s.bg(t.hover).text_color(t.foreground))
      .on_click(action).child(svg(picture).size(12)).child(label);
    const hint = "↑ ↓ or tab to move · Enter or Space to show · type to filter · Esc to close";
    // Invisible at first after alt-tab: a quick one shows the last space without a flash.
    const root = v_flex().id("spaces").track_focus(this.handle).size_full().min_h(0).text_size(13)
      .on_key_down((e, cx) => this.key(e, cx))
      .on_modifiers_changed((e) => this.released_(e));
    if (!this.visible) return root;
    // A popup is as tall as its content: the window sets its own height (the bar passes it).
    return root.h(props()?.height ?? 600).bg(t.popover).rounded(12).border_1().border_color(t.border).shadow_lg().overflow_hidden()
      .child(h_flex().h(44).flex_none().px(14).gap(8).items_center().border_b(1).border_color(t.border)
        .child(div().flex_1().text_size(13.5).font_semibold().text_color(t.foreground).child("Spaces"))
        .child(h_flex().gap(8)
          .child(button("new-space", "mods/workspace-chrome/plus.svg", "New space", () => {
            this.dismiss();
            overlay("helm-task", { mode: "new" });
          }))
          .child(button("settings", "mods/workspace-chrome/settings.svg", "Helm Settings", () => {
            this.dismiss();
            overlay("helm-projects");
          }))))
      .child(h_flex().flex_1().min_h(0).child(this.left(t)).child(this.details(t)))
      .child(div().h(28).flex_none().px(14).flex().items_center().border_t(1).border_color(t.border).text_size(11).text_color(t.dim).child(hint));
  }
}
