// Uncommitted changes of every repo of the shown space, from `herdr-helm changes`: a
// project space's checkouts, else the repo at the focused folder, or the repos under it.
// One group per repo with changes or unpushed commits; the rest fold into one line. A
// file opens its diff in the editor, and a group's "Open all" opens an editor window with a
// diff of each of its files. The editor connector does this, and without one these do not show. One message commits every ticked repo; a group's ↑ pushes it. The
// count of changed files goes to the Changes pill ("changes.count"). Commits and pushes run in the background: exec stops at 30 s.

import { View, div, svg } from "gpui-kit";
import { v_flex, h_flex, InputState, Input } from "gpui-base";
import { exec, home, watchFolder, theme } from "herdr";
import { helm, background, can, keep } from "./helm.js";
import { awake, location, checkBox } from "./ui.js";

const key = () => JSON.stringify(location());

const lastLine = (text) => (text ?? "").split("\n").filter(Boolean).pop() ?? "";

// A push of a branch without an upstream: to the remote that remote.pushDefault names, else to
// the remote of the main or master branch, else to the first remote. It sets the upstream.
const PUSH_NEW = 'r=$(git config --get remote.pushDefault); for b in main master; do [ -n "$r" ] && break; r=$(git config --get "branch.$b.remote"); done; [ -n "$r" ] || r=$(git remote | head -n 1); [ -n "$r" ] || { echo "This repo has no remote." >&2; exit 1; }; git push -u "$r" HEAD';

export default class Changes extends View {
  init(_props, cx) {
    this.repos = []; // herdr-helm changes: { repo, path, branch, upstream, ahead, behind, files }
    this.folded = new Set(); // repo paths
    this.picked = new Set(); // repo paths ticked for the next commit
    this.seen = new Set(); // repo paths that had changes at the last read: new ones start ticked
    this.notes = {}; // repo path -> the last commit or push result
    this.busy = null; // "commit", or the path being pushed
    this.message = "";
    this.input = InputState.new({ placeholder: "Commit message" });
    this.input.on("change", (_e, cx) => {
      const had = Boolean(this.message.trim());
      this.message = this.input.value();
      if (had !== Boolean(this.message.trim())) cx.notify();
    });
    this.input.on("submit", (_e, cx) => this.commit(cx));
    this.input.on("focus", (_e, cx) => { this.typing = true; cx.notify(); });
    this.input.on("blur", (_e, cx) => { this.typing = false; cx.notify(); });
    this.watched = new Set();
    this.refresh(cx);
    cx.timer.every(10000, (cx) => awake() && this.refresh(cx));
    // A file change while away only marks the list stale; back in front, it reads again.
    this.stale = false;
    let was = awake();
    cx.timer.every(1000, (cx) => { const now = awake(); if (now && !was && this.stale) { this.stale = false; this.refresh(cx); } was = now; });
  }

  refresh(cx) {
    if (this.loading) return;
    const where = location(), context = key();
    where.cwd ||= home();
    if (context !== this.context) {
      this.context = context; this.repos = []; this.error = null; this.notes = {}; this.seen = new Set(); this.picked = new Set();
    }
    this.loading = true;
    cx.spawn(async (cx) => {
      try {
        const r = await helm(["changes", ...(where.space ? ["--herdr-space", where.space] : []), "--folder", where.cwd]);
        if (key() !== context) return;
        if (!r.data) throw new Error(r.error);
        this.repos = r.data.repos;
        this.on = where.on; // the machine of the git calls
        this.error = null;
        const changed = new Set(this.repos.filter((x) => x.files.length).map((x) => x.path));
        for (const p of changed) if (!this.seen.has(p)) this.picked.add(p);
        for (const p of [...this.picked]) if (!changed.has(p)) this.picked.delete(p);
        this.seen = changed;
        for (const x of this.repos) this.watch(x.path, cx);
        // Every mod renders again on a shared change: write only a new count.
        const count = { space: where.space, files: this.repos.reduce((n, x) => n + x.files.length, 0), ahead: this.repos.reduce((n, x) => n + x.ahead, 0) };
        keep("changes.count", count);
      } catch (e) {
        if (key() === context) this.error = String(e.message ?? e);
      } finally { this.loading = false; cx.notify(); }
    });
  }

  watch(root, cx) {
    if (this.watched.has(root)) return;
    this.watched.add(root);
    cx.spawn(async (cx) => {
      try { for (;;) { await watchFolder(root); if (awake()) this.refresh(cx); else this.stale = true; } }
      catch { this.watched.delete(root); }
    });
  }

  // The editor's diff of the file against HEAD (`herdr-helm open --change`).
  diffInCode(x, f) {
    helm(["open", "--change", x.path, `${f.status}:${f.path}`]);
  }

  // A new editor window on the repo, then a diff tab per file in the panel's order
  // (`herdr-helm open --changes`, which runs in the background). A new folder (untracked) has
  // no single diff: it is left out.
  diffAllInCode(x) {
    const files = x.files.filter((f) => !f.path.endsWith("/")).map((f) => `${f.status}:${f.path}`);
    if (!files.length) return;
    helm(["open", "--changes", x.path, ...files]);
    this.notes[x.path] = { ok: true, text: `Opening ${files.length} ${files.length === 1 ? "diff" : "diffs"} in an editor window` };
  }

  // `git add -A` and `git commit` with one message in every ticked repo, one by one; a
  // failure in one repo leaves the others to go on.
  commit(cx) {
    const message = this.message.trim();
    const targets = this.repos.filter((x) => this.picked.has(x.path) && x.files.length);
    if (!message || !targets.length || this.busy) return;
    this.busy = "commit";
    cx.notify();
    cx.spawn(async (cx) => {
      let failed = 0;
      try {
        for (const x of targets) {
          const job = (argv) => background(argv, { cwd: x.path, on: this.on });
          const add = await job(["git", "add", "-A"]);
          const r = add.code === 0 ? await job(["git", "commit", "-m", message]) : add;
          if (r.code === 0) this.notes[x.path] = { ok: true, text: "Committed" };
          else { failed++; this.notes[x.path] = { ok: false, text: lastLine(r.stderr || r.stdout) || "The commit failed." }; }
          cx.notify();
        }
        if (!failed) { this.message = ""; this.input.set_value(""); }
      } finally {
        // Also after an error: the panel must not stay at "Committing…".
        this.busy = null;
        this.refresh(cx);
      }
    });
  }

  // Undoes one file's uncommitted changes: a tracked file goes back to HEAD (staged
  // changes too), a new file is deleted. It cannot be taken back, so it takes two clicks. A
  // file with a merge conflict has no undo: resolve the conflict with git.
  undo(x, f, cx) {
    if (f.status === "U") return;
    const id = `${x.path}/${f.path}`;
    if (this.armed !== id) { this.armed = id; return cx.notify(); }
    this.armed = null;
    cx.spawn(async (cx) => {
      const fail = (e) => ({ code: -1, stdout: "", stderr: String(e) });
      const git = (args) => exec("git", ["-C", x.path, ...args], { on: this.on }).catch(fail);
      let r;
      if (f.untracked) r = await git(["clean", "-f", "--", f.path]);
      else if (f.status === "A") {
        r = await git(["rm", "--cached", "--quiet", "--", f.path]);
        if (r.code === 0) r = await exec("rm", ["-f", "--", id], { on: this.on }).catch(fail);
      } else r = await git(["restore", "--source=HEAD", "--staged", "--worktree", "--", f.path]);
      this.notes[x.path] = r.code === 0 ? { ok: true, text: `Undid ${f.path}` } : { ok: false, text: lastLine(r.stderr || r.stdout) || "The undo failed." };
      this.refresh(cx);
    });
    cx.notify();
  }

  push(x, cx) {
    if (this.busy) return;
    this.busy = x.path;
    cx.notify();
    cx.spawn(async (cx) => {
      try {
        // With an upstream, git pushes to it. Without one, see PUSH_NEW: no remote name is assumed.
        const r = await background(x.upstream ? ["git", "push"] : ["sh", "-c", PUSH_NEW], { cwd: x.path, on: this.on });
        this.notes[x.path] = r.code === 0 ? { ok: true, text: "Pushed" } : { ok: false, text: lastLine(r.stderr) || "The push failed." };
      } finally {
        this.busy = null;
        this.refresh(cx);
      }
    });
  }

  group(t, x) {
    const folded = this.folded.has(x.path);
    const add = x.files.reduce((n, f) => n + f.add, 0), del = x.files.reduce((n, f) => n + f.del, 0);
    const note = this.notes[x.path];
    const head = h_flex().id(`repo-${x.path}`).h(30).flex_none().px(6).gap(7).items_center().rounded(7).cursor_pointer().hover((s) => s.bg(t.hover))
      .on_click((_e, cx) => { folded ? this.folded.delete(x.path) : this.folded.add(x.path); cx.notify(); })
      .when(x.files.length > 0, (el) => el.child(div().id(`pick-${x.path}`)
        .on_click((_e, cx) => { cx.stop_propagation(); this.picked.has(x.path) ? this.picked.delete(x.path) : this.picked.add(x.path); cx.notify(); })
        .child(checkBox(t, this.picked.has(x.path)))))
      .child(svg(`mods/workspace-chrome/chevron-${folded ? "right" : "down"}.svg`).size(11).flex_none().text_color(t.dim))
      .child(div().flex_none().max_w(150).truncate().font_semibold().child(x.repo))
      .child(div().flex_1().min_w(0).truncate().text_size(11.5).text_color(t.dim).child(x.branch ?? ""))
      .when(add > 0, (el) => el.child(div().text_size(11).text_color(t.ansi2).child(`+${add}`)))
      .when(del > 0, (el) => el.child(div().text_size(11).text_color(t.ansi1).child(`−${del}`)))
      .when(x.files.length > 0 && can("editor", "window"), (el) => el.child(div().id(`open-all-${x.path}`).px(6).h(20).flex().items_center().rounded(5).text_size(11)
        .text_color(t.muted).hover((s) => s.bg(t.hover).text_color(t.foreground))
        .on_click((_e, cx) => { cx.stop_propagation(); this.diffAllInCode(x); cx.notify(); })
        .child("Open all")))
      .when(x.ahead > 0, (el) => el.child(div().id(`push-${x.path}`).px(6).h(20).flex().items_center().rounded(5).text_size(11)
        .bg(t.accent + "26").text_color(t.accent).hover((s) => s.bg(t.accent + "40"))
        .on_click((_e, cx) => { cx.stop_propagation(); this.push(x, cx); })
        .child(this.busy === x.path ? "Pushing…" : `↑${x.ahead}`)));
    const rows = folded ? [] : x.files.map((f) => {
      const parts = f.path.split("/"), name = parts.pop();
      const armed = this.armed === `${x.path}/${f.path}`;
      return h_flex().id(`change-${x.path}/${f.path}`).h(26).flex_none().pl(28).pr(6).gap(8).items_center().rounded(7).cursor_pointer().hover((s) => s.bg(t.hover))
        .on_click((_e, cx) => { this.armed = null; if (can("editor", "diff")) this.diffInCode(x, f); cx.notify(); })
        .child(div().w(12).text_size(11).text_color(f.status === "A" ? t.ansi2 : f.status === "D" || f.status === "U" ? t.ansi1 : t.blocked).child(f.status))
        .child(h_flex().flex_1().min_w(0).child(div().text_color(t.dim).truncate().child(parts.length ? parts.join("/") + "/" : "")).child(div().flex_none().max_w(220).truncate().child(name)))
        .when(f.add > 0, (el) => el.child(div().text_size(11).text_color(t.ansi2).child(`+${f.add}`)))
        .when(f.del > 0, (el) => el.child(div().text_size(11).text_color(t.ansi1).child(`−${f.del}`)))
        .when(f.status !== "U", (el) => el.child(div().id(`undo-${x.path}/${f.path}`).flex_none().px(5).h(18).flex().items_center().rounded(4).text_size(11)
          .when(armed, (el) => el.bg(t.ansi1 + "26").text_color(t.ansi1).font_semibold())
          .when(!armed, (el) => el.text_color(t.dim).hover((s) => s.bg(t.hover).text_color(t.foreground)))
          // Its press must not reach the panel, which cancels an armed undo.
          .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
          .on_click((_e, cx) => { cx.stop_propagation(); this.undo(x, f, cx); })
          .child(armed ? "Undo?" : "↺")));
    });
    return [head, ...(note ? [div().pl(28).pb(2).text_size(11).text_color(note.ok ? t.ansi2 : t.ansi1).truncate().child(note.text)] : []), ...rows];
  }

  render(cx) {
    if (key() !== this.context) this.refresh(cx);
    const t = theme();
    const shown = this.repos.filter((x) => x.files.length || x.ahead > 0);
    const quiet = this.repos.length - shown.length;
    const files = shown.reduce((n, x) => n + x.files.length, 0);
    const targets = this.repos.filter((x) => this.picked.has(x.path) && x.files.length).length;
    const enabled = Boolean(this.message.trim()) && targets > 0 && !this.busy;
    return v_flex().id("changes").on_mouse_down("left", (_e, cx) => { if (this.armed) { this.armed = null; cx.notify(); } }).size_full().min_h(0).rounded(10).overflow_hidden().bg(t.background).border_1().border_color(t.border).text_color(t.foreground).text_size(13)
      // The header of a tile: 30 px, 12 px text.
      .child(h_flex().h(30).flex_none().px(10).gap(8).items_center().border_b(1).border_color(t.border).text_size(12)
        .child(div().font_semibold().child("Changes"))
        .child(div().px(5).py(1).rounded(4).border_1().border_color(t.border).text_size(10).text_color(t.muted).child(`${files} files`))
        .child(div().flex_1())
        .child(div().text_size(11).text_color(t.dim).child(`${this.repos.length} ${this.repos.length === 1 ? "repo" : "repos"}`)))
      .child(v_flex().id("changes-list").flex_1().min_h(0).overflow_y_scrollbar().p(6)
        .children(shown.flatMap((x) => this.group(t, x)))
        .when(quiet > 0, (el) => el.child(div().px(8).py(6).text_size(11).text_color(t.dim)
          .child(`${quiet} ${quiet === 1 ? "repo" : "repos"} with no changes`)))
        .when(!shown.length, (el) => el.child(div().p(8).text_color(t.dim)
          .child(this.error || (this.loading && !this.repos.length ? "Loading changes…" : this.repos.length ? "No uncommitted changes" : "No git repos here")))))
      .child(h_flex().flex_none().p(10).gap(8).border_t(1).border_color(t.border)
        .child(div().id("commit-message").flex_1().min_w(0).h(32).px(10).flex().items_center().rounded(8).border_1().border_color(this.typing ? t.accent : t.border).cursor_text()
          .on_mouse_down("left", (_e, cx) => cx.stop_propagation()).child(Input.new(this.input).h_full().text_size(12).text_color(t.foreground)))
        .child(div().id("commit").h(32).px(14).flex().items_center().rounded(8).bg(enabled ? t.solid : t.raised).text_color(enabled ? t.solid_foreground : t.dim).text_size(12)
          .when(enabled, (e) => e.cursor_pointer()).on_click((_e, cx) => this.commit(cx))
          .child(this.busy === "commit" ? "Committing…" : targets > 1 ? `Commit ${targets} repos` : "Commit")));
  }
}
