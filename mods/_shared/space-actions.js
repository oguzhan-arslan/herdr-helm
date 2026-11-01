// The actions of a space as buttons, for the top bar and the spaces window of helm.
//   Project space (herdr-helm): Rename, Smart rename, Complete, Delete.
//   Plain space: Rename, Smart rename, the worktree actions of its checkout, Close.
// Shared: the installer copies mods/_shared into every Helm mod. Edit it here only.

import { div, svg } from "gpui-kit";
import { h_flex } from "gpui-base";
import { run, setShared, shared, spaces, exec } from "herdr";
import { overlay } from "./helm.js";
import { place } from "./ui.js";

const icon = (name) => `mods/workspace-chrome/${name}.svg`;
const checkouts = {}; // space -> "main" | "linked" | null: its folder's git checkout

// Looks at a space's checkout once (two quick git calls in the background). Call it from a
// timer or an event, not from render; spaceActions reads the result.
export function lookAtCheckout(id, cx) {
  const s = spaces().find((x) => x.id === id);
  if (!s || s.id in checkouts) return;
  checkouts[s.id] = null;
  const folder = place(s);
  if (folder) cx.spawn(async (cx) => {
    const top = await exec("git", ["rev-parse", "--show-toplevel"], { cwd: folder, on: s.machine }).catch(() => null);
    const common = await exec("git", ["rev-parse", "--path-format=absolute", "--git-common-dir"], { cwd: folder, on: s.machine }).catch(() => null);
    checkouts[s.id] = top?.code === 0 && common?.code === 0 ? (common.stdout.trim() === `${top.stdout.trim()}/.git` ? "main" : "linked") : null;
    cx.notify();
  });
}

// A Smart rename of the space runs (helm-jobs says so in "smart.running"), or was asked for a
// moment ago and has not started yet. A run that hangs stops showing after 2 minutes.
function smartBusy(id) {
  const started = (shared("smart.running") ?? {})[id];
  const asked = shared("smart.request");
  return (Boolean(started) && Date.now() - started < 120000) || (asked?.id === id && Date.now() - asked.at < 3000);
}

const task = (mode, space) => () => overlay("helm-task-end", { mode, space });
const worktree = (space, action) => () => overlay(action === "open" ? "worktree-open" : "worktree", { space, action });

// [{ label, icon, act, danger?, hint?, busy?, stay? }] for space `id`.
export function spaceActions(id) {
  const s = spaces().find((x) => x.id === id);
  if (!s) return [];
  const common = [
    { label: "Rename", icon: icon("square-pen"), act: () => run("space.rename", { id }) },
    { label: smartBusy(id) ? "Smart rename runs…" : "Smart rename", icon: "mods/helm-bar/sparkles.svg", busy: smartBusy(id),
      act: () => setShared("smart.request", { id, at: Date.now() }), stay: true },
  ];
  const t = (shared("helm.spaces") ?? []).find((x) => x.herdr_space === id);
  if (t) {
    // A space whose PRs are all no longer open (merged or closed) is likely done.
    const live = new Set((shared("prs.data")?.sections ?? []).flatMap((x) => x.prs).map((p) => p.key));
    const done = (t.prs ?? []).length > 0 && Boolean(shared("prs.data")) && !t.prs.some((p) => live.has(p.key));
    return [...common,
      { label: done ? "Complete: its PRs are merged or closed" : "Complete", icon: icon("circle-check"), act: task("complete", t.name), hint: done },
      { label: "Delete", icon: icon("archive"), act: task("delete", t.name), danger: true }];
  }
  const git = checkouts[id] ?? null;
  return [...common,
    ...(git ? [{ label: "New worktree", icon: icon("git-branch"), act: worktree(id, "new") }, { label: "Open worktree", icon: icon("folder"), act: worktree(id, "open") }] : []),
    ...(git === "linked" ? [{ label: "Remove worktree", icon: icon("minus"), act: worktree(id, "remove") }] : []),
    { label: "Close", icon: icon("x"), act: () => run("space.close", { id }), danger: true }];
}

// The actions as small icon buttons with their name on hover. `before` runs first (a dialog
// closes itself before the action opens its own), except for an action that opens nothing
// (`stay`: Smart rename), so its spinner shows there.
export function actionButtons(t, id, before = () => {}) {
  return h_flex().flex_none().gap(2).items_center()
    .children(spaceActions(id).map((a, i) => a.busy
      // Running: a spinner in its place, and a click does nothing.
      ? div().id(`space-action-${id}-${i}`).size(26).flex_none().flex().items_center().justify_center().tooltip(a.label)
        .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
        .child(svg("mods/workspace-chrome/spinner.svg").size(14).text_color(t.accent).spin(900))
      : div().id(`space-action-${id}-${i}`).size(26).flex_none().flex().items_center().justify_center().rounded(6)
        .text_color(a.hint ? t.ansi2 : t.dim)
        .hover((s) => s.bg(a.danger ? t.blocked + "26" : t.hover).text_color(a.danger ? t.blocked : t.foreground))
        .tooltip(a.label)
        .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
        .on_click((_e, cx) => { cx.stop_propagation(); if (!a.stay) before(); a.act(); })
        .child(svg(a.icon).size(14))));
}
