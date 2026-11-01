// The org's open PRs in the GitHub popup of helm, in sections by what they need from you, with
// actions per PR on the hovered card. Review and Fix open an agent tab in the PR's own space
// (`herdr-helm pr`); nothing is pushed or posted without the person's go. The actions show only
// when the code host connector can do them. helm-jobs reads the
// PRs into shared "prs.data"; Refresh asks it again ("prs.refresh").

import { View, div, svg } from "gpui-kit";
import { v_flex, h_flex, Scrollbar } from "gpui-base";
import { theme, shared, setShared, props, closeOverlay, notify } from "herdr";
import { helmSlow, can } from "./helm.js";
import { ago, agoText } from "./ui.js";


const FRESH = 60000; // the popup reads again when the data is older than this

const task = helmSlow;

const SECTION_COLOR = (t) => ({ attention: t.ansi1, approved: t.ansi2, queue: t.ansi5, review: t.ansi4 });
const QUEUE_REASON = { stack_invalidated: "stack changed", failed_checks: "checks failed", merge_conflict: "conflict" };

// Sections folded until the person opens them: the ones that rarely need you.
const FOLDED = ["other", "drafts"];

// helm-jobs reads them again.
const refresh = () => setShared("prs.refresh", Date.now());

export default class Prs extends View {
  init(_props, cx) {
    this.busy = {}; // PR key -> the running action
    this.armed = null; // PR key armed for Add to queue
    this.hovered = null; // PR key under the pointer: only its card shows its actions
    this.notes = {}; // PR key -> the last action result
    try { this.folded = new Set(JSON.parse(localStorage.getItem("folded") ?? "null") ?? FOLDED); }
    catch { this.folded = new Set(FOLDED); }
    const data = shared("prs.data");
    if (!data || Date.now() - data.at > FRESH) refresh();
  }

  // Runs a herdr-helm pr action; Review and Fix close the popup once their tab opens.
  act(pr, action, cx) {
    if (this.busy[pr.key]) return;
    this.busy[pr.key] = action;
    this.notes[pr.key] = null;
    cx.notify();
    const args = ["pr", action, pr.repo, String(pr.number), ...(action === "fix" ? ["--reasons", pr.reasons.join(", ") || "see the PR"] : [])];
    cx.spawn(async (cx) => {
      const r = await task(args);
      delete this.busy[pr.key];
      if (!r.ok) this.notes[pr.key] = { ok: false, text: r.error || `${action} failed` };
      else if (action === "enqueue") { this.notes[pr.key] = { ok: true, text: "Added to the merge queue" }; refresh(); }
      else {
        const verb = { fix: "Fixing", review: "Reviewing" }[action];
        notify({ title: `${verb} ${pr.repo}#${pr.number}`, body: `In space ${r.data.space}. The agent stops when it is ready for you.`, level: "info" });
        return closeOverlay(props().overlay);
      }
      cx.notify();
    });
  }

  // An icon button of the hover bar. It keeps its click from the card, which opens the PR.
  iconButton(t, id, icon, tip, onClick, color) {
    const c = color ?? t.accent;
    return div().id(id).size(30).flex_none().flex().items_center().justify_center().rounded(7).cursor_pointer()
      .bg(c + "1f").text_color(c).hover((s) => s.bg(c + "40"))
      .tooltip(tip)
      .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
      .on_click((_e, cx) => { cx.stop_propagation(); onClick(cx); })
      .child(svg(`mods/helm-prs/${icon}.svg`).size(16));
  }

  // The actions of one PR. They float over the right side of the card, so the card never
  // changes size when they show.
  actions(t, pr, section) {
    const busy = this.busy[pr.key];
    const armed = this.armed === pr.key;
    const bar = h_flex().gap(4).p(4).items_center().rounded(9).bg(t.popover).border(1).border_color(t.border);
    if (busy) return bar.px(10).h(40).gap(8)
      .child(svg("mods/workspace-chrome/spinner.svg").size(14).text_color(t.dim).spin(900))
      .child(div().text_size(11.5).text_color(t.muted).child(busy === "enqueue" ? "Queueing…" : "Opening…"));
    return bar
      .when(pr.mine && section === "attention" && can("code_host", "fix"), (el) => el.child(this.iconButton(t, `fix-${pr.key}`, "fix", "Fix: an agent prepares a fix in the PR's space, without a push", (cx) => this.act(pr, "fix", cx), t.ansi1)))
      .when((!pr.mine || section !== "attention") && can("code_host", "review"), (el) => el.child(this.iconButton(t, `review-${pr.key}`, "review", "Review: an agent reviews the PR in its own space", (cx) => this.act(pr, "review", cx))))
      .when(section === "approved" && can("code_host", "merge_queue"), (el) => el.child(this.iconButton(t, `queue-${pr.key}`, armed ? "confirm" : "enqueue",
        armed ? "Click again to add the PR to the merge queue. Others see this." : "Add to the merge queue",
        (cx) => { if (!armed) { this.armed = pr.key; return cx.notify(); } this.armed = null; this.act(pr, "enqueue", cx); }, t.ansi2)))
      // The code host connector gives the link of the queue. Without one, no link shows.
      .when((section === "queue" || Boolean(pr.queueRemoval)) && Boolean(pr.queue_url), (el) => el.child(this.iconButton(t, `mq-${pr.key}`, "queue", "Open the merge queue", (cx) => cx.open_url(pr.queue_url), t.muted)));
  }

  row(t, pr, section) {
    const chip = (text, color) => div().flex_none().px(5).h(16).flex().items_center().rounded(4).text_size(10).bg(color + "26").text_color(color).child(text);
    const ci = pr.ci === "SUCCESS" ? t.ansi2 : pr.ci === "FAILURE" || pr.ci === "ERROR" ? t.ansi1 : pr.ci ? t.ansi3 : null;
    const note = this.notes[pr.key];
    const hovered = this.hovered === pr.key;
    const showActions = hovered || Boolean(this.busy[pr.key]) || this.armed === pr.key;
    // Indented under its section header, so it is clear which section a PR is in.
    return v_flex().id(`pr-${pr.key}`).relative().flex_none().ml(20).mt(6).px(10).py(8).gap(4).rounded(8).cursor_pointer()
      .border(1).border_color(hovered ? t.accent + "80" : t.border).when(hovered, (el) => el.bg(t.hover))
      .on_hover((on, cx) => {
        if (on) this.hovered = pr.key;
        else if (this.hovered === pr.key) this.hovered = null;
        cx.notify();
      })
      .on_click((_e, cx) => cx.open_url(pr.url))
      .child(h_flex().gap(8).items_start()
        .child(div().id(`title-${pr.key}`).flex_1().min_w(0).text_size(13).font_medium().text_color(t.foreground).child(pr.title))
        .child(div().flex_none().pt(1).text_size(10.5).text_color(t.dim).child(ago(pr.updatedAt))))
      .child(h_flex().gap(6).items_center().flex_wrap()
        .when(Boolean(ci), (el) => el.child(div().id(`ci-${pr.key}`).size(7).flex_none().rounded(4).bg(ci)))
        .child(div().flex_none().text_size(11).text_color(t.dim).child(`${pr.repo}#${pr.number}`))
        .child(div().flex_none().text_size(11).text_color(t.dim).child("·"))
        .child(div().flex_none().text_size(11).text_color(t.muted).child(pr.author))
        .children(pr.reasons.map((r) => chip(r, t.ansi1)))
        .when(Boolean(pr.queueRemoval), (el) => el.child(chip(`Removed from queue: ${QUEUE_REASON[pr.queueRemoval] ?? pr.queueRemoval}`, t.ansi1)))
        .when(Boolean(pr.queue), (el) => el.child(chip(`Queue #${pr.queue.position}`, t.ansi5)))
        .when(Boolean(pr.stack), (el) => el.child(chip(`Stack ${pr.stack}`, t.dim)))
        .when(pr.isDraft, (el) => el.child(chip("Draft", t.dim)))
        .when(pr.requestedMe, (el) => el.child(chip("Asks you", t.ansi4))))
      .when(Boolean(note), (el) => el.child(div().text_size(11).text_color(note.ok ? t.ansi2 : t.ansi1).child(note.text)))
      .when(showActions, (el) => el.child(h_flex().absolute().top(0).bottom(0).right(8).items_center()
        .child(this.actions(t, pr, section))));
  }

  // The GitHub budgets a refresh spends from: what is left of each, and when it resets.
  limits(t, rate) {
    const clock = (s) => new Date(s * 1000).toTimeString().slice(0, 5);
    const item = (label, r) => {
      const left = r.remaining / r.limit;
      const color = left < 0.05 ? t.ansi1 : left < 0.2 ? t.ansi3 : t.ansi2;
      return h_flex().id(`limit-${label}`).gap(6).items_center()
        .child(div().text_size(10.5).text_color(t.dim).child(label))
        .child(div().relative().w(60).h(4).rounded(2).bg(t.border)
          .child(div().absolute().left(0).top(0).h(4).rounded(2).w(Math.max(2, 60 * left)).bg(color)))
        .child(div().text_size(10.5).text_color(left < 0.2 ? color : t.muted).child(`${r.remaining.toLocaleString()} left \u00b7 ${clock(r.reset)}`));
    };
    return h_flex().px(8).pb(6).gap(16).items_center()
      .child(item("GraphQL", rate.graphql))
      .when(Boolean(rate.core), (el) => el.child(item("REST", rate.core)));
  }

  popup(t) {
    const data = shared("prs.data"), error = shared("prs.error");
    const colors = SECTION_COLOR(t);
    return v_flex().id("prs-popup").size_full().p(6).bg(t.popover).text_size(12.5)
      .on_mouse_down("left", (_e, cx) => { if (this.armed) { this.armed = null; cx.notify(); } })
      .child(h_flex().px(8).pt(2).pb(6).gap(8).items_center()
        .child(svg("mods/helm-bar/github.svg").size(13).text_color(t.muted))
        .child(div().flex_1().text_size(11).text_color(t.dim)
          .child(data ? `Open PRs in ${data.org} · updated ${agoText(data.at)}` : "Open PRs"))
        .when(Boolean(shared("prs.loading")), (el) => el.child(svg("mods/workspace-chrome/spinner.svg").size(11).text_color(t.dim).spin(900)))
        .child(div().id("prs-refresh").px(6).rounded(5).cursor_pointer().text_size(11).text_color(t.muted).hover((s) => s.bg(t.hover))
          .on_click(() => refresh()).child("Refresh")))
      .when(Boolean(data?.rate?.graphql), (el) => el.child(this.limits(t, data.rate)))
      .when(Boolean(error), (el) => el.child(div().px(8).pb(6).text_size(11).text_color(t.ansi1).child(error)))
      .when(!data, (el) => el.child(div().p(8).text_color(t.dim).child("Reading the PRs…")))
      .child(div().relative().flex_1().min_h(0)
        .child(v_flex().id("prs-list").size_full().overflow_y_scroll()
          .children((data?.sections ?? []).flatMap((s, i) => {
            const folded = this.folded.has(s.key);
            return [
              h_flex().id(`sec-${s.key}`).when(i > 0, (el) => el.mt(10)).px(8).pt(10).pb(4).gap(6).items_center().rounded(6).cursor_pointer()
                .hover((el) => el.bg(t.hover))
                .on_click((_e, cx) => {
                  folded ? this.folded.delete(s.key) : this.folded.add(s.key);
                  try { localStorage.setItem("folded", JSON.stringify([...this.folded])); } catch {}
                  cx.notify();
                })
                .child(svg(`mods/workspace-chrome/chevron-${folded ? "right" : "down"}.svg`).size(11).flex_none().text_color(t.dim))
                .child(div().size(7).rounded(4).bg(colors[s.key] ?? t.dim))
                .child(div().font_semibold().text_size(12).child(s.title))
                .child(div().text_size(11).text_color(t.dim).child(String(s.prs.length))),
              ...(folded ? [] : s.prs.map((pr) => this.row(t, pr, s.key))),
            ];
          })))
        .child(Scrollbar.vertical("prs-list").absolute().inset_0()));
  }

  render() {
    const t = theme();
    return this.popup(t);
  }
}
