// Project spaces through herdr-helm: props { mode: "new", project? } makes a space with a
// checkout per chosen repo; { mode: "complete" | "delete", space } ends one.
// "Other" in the project row (props { mode: "new", other: true } picks it) makes a plain Herdr
// space in a folder: one of the folders agent sessions ran in, or one you choose. Long steps
// (a fetch and a checkout of a large repo) run in the background: exec stops at 30 s.

import { View, div } from "gpui-kit";
import { v_flex, h_flex, InputState, Input, TextareaState, Textarea, Scrollbar } from "gpui-base";
import { props, closeOverlay, theme, home, appHome, readTextFile, shared, run, pickFolder } from "herdr";
import { helm, helmSlow, STATE, keep } from "./helm.js";
import { checkBox, shortHome } from "./ui.js";

// The agent connector's name for the first tab, or null when no agent can start.
const AGENT = () => {
  const a = shared("helm.connectors")?.agent;
  return a && (a.capabilities ?? []).includes("start") ? a.label ?? a.name : a ? null : "Agent";
};

// The name and branch herdr-helm makes from the title (its slugify): shown while typing.
// herdr-helm adds -2 when that name is taken.
function slugify(text) {
  const words = text.normalize("NFKD").replace(/[\u0300-\u036f]/g, "").toLowerCase().split(/[^a-z0-9]+/).filter(Boolean);
  let name = "";
  for (const w of words) {
    if (name.length + w.length + (name ? 1 : 0) > 23) break;
    name = name ? `${name}-${w}` : w;
  }
  name = name || (words[0] ?? "space").slice(0, 23);
  return ["main", "master", "develop", "head"].includes(name) ? `${name.slice(0, 18)}-work` : name;
}
// herdr-helm refuses complete and delete in a terminal, so agents cannot end a space. This
// dialog is Herdr, so helm.js calls it without the terminal's id.
const quick = helm;
const slow = helmSlow;

export default class Task extends View {
  init(_props, cx) {
    this.mode = props()?.mode ?? "new";
    this.busy = false;
    this.error = "";
    this.handle = cx.focus_handle();
    if (this.mode === "new") return this.initNew(cx);
    this.handle.focus();
    this.check = undefined; // complete: { ok, problems }; delete: { losses }
    cx.spawn(async (cx) => {
      const r = await quick(this.mode === "complete" ? ["complete", "--check", "--space", props().space] : ["delete", "--space", props().space]);
      this.check = r.data ?? { problems: [{ checkout: "", what: r.error || "herdr-helm did not answer" }] };
      cx.notify();
    });
  }

  initNew(cx) {
    this.projects = undefined;
    this.project = null;
    this.other = Boolean(props()?.other);
    this.folder = null;
    this.folders = this.knownFolders();
    this.readFolders(cx);
    this.picked = new Set();
    this.order = [];
    this.info = {}; // repo name: { base, branch, host } from herdr-helm project show
    this.agent = AGENT() ? "agent" : "shell";
    this.progress = "";
    this.name = InputState.new({ placeholder: "What are you working on? e.g. SSO login times out" });
    this.filter = InputState.new({ placeholder: "Filter repos" });
    this.goal = TextareaState.new({ placeholder: `A first message for ${AGENT() ?? "the agent"} (optional). Empty: it starts without one.` });
    this.goal.set_auto_grow(3, 6);
    this.focused = "name";
    // ↵ in the title creates the space. The goal takes more lines: ⌘↵ creates from there.
    for (const [key, input] of [["name", this.name], ["filter", this.filter], ["goal", this.goal]]) {
      input.on("change", (_e, cx) => { this.error = ""; cx.notify(); });
      input.on("focus", (_e, cx) => { this.focused = key; cx.notify(); });
    }
    this.name.on("submit", (_e, cx) => this.create(cx));
    this.name.focus();
    cx.timer.after(600, () => this.name.focus());
    // The projects the sidebar read last show at once. The read below brings them up to date.
    const pick = (cx) => {
      if (this.other) return;
      this.choose(this.projects.find((p) => p.name === props()?.project) ?? this.projects[0], cx);
    };
    if (Array.isArray(shared("helm.projects"))) {
      this.projects = shared("helm.projects");
      pick(cx);
    }
    cx.spawn(async (cx) => {
      const r = await quick(["projects"]);
      if (!r.data) this.error = r.error || "herdr-helm did not answer";
      else keep("helm.projects", r.data);
      const chosen = this.project?.name;
      this.projects = r.data ?? this.projects ?? [];
      // Keeps the ticks of a project already on screen. Only its data changes.
      const same = this.projects.find((p) => p.name === chosen);
      if (same) this.project = same;
      else pick(cx);
      cx.notify();
    });
  }

  // Folders for a plain space, newest first: kept here, so a folder stays after its session ends.
  knownFolders() {
    try { return JSON.parse(localStorage.getItem("folders") ?? "[]"); } catch { return []; }
  }

  keepFolder(path, at) {
    const h = home();
    // Project space checkouts and temporary folders are no place for a plain space.
    const inside = (root) => Boolean(root) && (path === root || path.startsWith(`${root}/`));
    if (!path || path === h || (this.roots ?? []).some(inside) || /^\/(private\/)?(tmp|var)\//.test(path)) return;
    const old = this.folders.find((f) => f.path === path);
    if (old) old.at = Math.max(old.at, at);
    else this.folders.push({ path, at });
    this.folders.sort((a, b) => b.at - a.at);
    this.folders = this.folders.slice(0, 20);
    localStorage.setItem("folders", JSON.stringify(this.folders));
  }

  // The folders of the agent sessions in Herdr terminals (`herdr-helm sessions save` writes
  // them) and Herdr's recent spaces. First the tasks folder (`herdr-helm settings`), whose
  // folders are left out, and the folders of the known project spaces.
  readFolders(cx) {
    this.roots = (shared("helm.spaces") ?? []).map((x) => x.path).filter(Boolean);
    cx.spawn(async (cx) => {
      const settings = await helm(["settings"]);
      const root = settings.data?.tasks_root;
      if (typeof root === "string" && root) this.roots.push(root.replace(/^~(?=\/|$)/, home()));
      const read = async (path) => { try { return JSON.parse((await readTextFile(path)) ?? "null"); } catch { return null; } };
      const [sessions, view] = await Promise.all([read(`${STATE()}/sessions.json`), read(`${appHome()}/view.json`)]);
      const now = Date.now();
      (view?.recent ?? []).forEach((r, i) => this.keepFolder(r.folder, now - 86400000 - i));
      for (const x of Object.values(sessions ?? {})) this.keepFolder(x.cwd, x.at ?? now);
      this.folder ??= this.folders[0]?.path ?? null;
      cx.notify();
    });
  }

  // A plain space in the chosen folder, with a shell, or with the agent and the goal as its first
  // message when one is written, as a project space does.
  createPlain(title, cx) {
    if (!this.folder) {
      this.error = "Pick a folder.";
      return cx.notify();
    }
    const goal = this.goal.value().trim();
    const start = shared("helm.connectors")?.agent?.start;
    const agent = this.agent === "agent" && start?.length ? { argv: start, ...(goal ? { task: goal } : {}) } : {};
    run("space.new", { name: title, folder: this.folder, ...agent });
    this.keepFolder(this.folder, Date.now());
    closeOverlay(props().overlay);
  }

  // The defaults come first. The order stays while the boxes change, so rows do not jump.
  choose(p, cx) {
    this.other = false;
    this.project = p ?? null;
    this.picked = new Set(p?.defaults ?? []);
    this.order = [...(p?.repos ?? [])].sort((a, b) => Number(this.picked.has(b)) - Number(this.picked.has(a)));
    if (!p) return;
    const info = (data) => Object.fromEntries((data?.repos ?? []).map((x) => [x.name, x]));
    this.info = info(shared(`helm.project.${p.name}`));
    cx.spawn(async (cx) => {
      const r = await quick(["project", "show", p.name]);
      if (r.data) keep(`helm.project.${p.name}`, r.data);
      if (this.project?.name !== p.name) return;
      this.info = info(r.data);
      cx.notify();
    });
  }

  close() {
    if (!this.busy) closeOverlay(props().overlay);
  }

  create(cx) {
    const title = this.name.value().trim();
    if (this.busy || (!this.project && !this.other)) return;
    if (!title) {
      this.error = "Say in a few words what you are working on.";
      return cx.notify();
    }
    if (this.other) return this.createPlain(title, cx);
    if (this.picked.size === 0) {
      this.error = "Pick at least one repo.";
      return cx.notify();
    }
    this.busy = true;
    this.error = "";
    cx.notify();
    const repos = [...this.picked].flatMap((r) => ["--repo", r]);
    const goal = this.agent === "agent" ? this.goal.value().trim() : "";
    this.progress = "";
    cx.spawn(async (cx) => {
      const r = await slow(["new", "--title", title, "--project", this.project.name, ...repos, ...(goal ? ["--goal", goal] : []),
        "--agent", this.agent, "--progress", "--open"], (line) => { this.progress = line; cx.notify(); });
      this.busy = false;
      if (r.code === 0) return closeOverlay(props().overlay);
      this.error = r.data?.error || r.error || `herdr-helm failed (exit ${r.code})`;
      cx.notify();
    });
  }

  end(cx) {
    const ready = this.mode === "complete" ? this.check?.ok : this.check?.losses;
    if (this.busy || !ready) return;
    this.busy = true;
    cx.notify();
    cx.spawn(async (cx) => {
      const r = await slow([this.mode, "--space", props().space, "--close", ...(this.mode === "delete" ? ["--yes"] : [])]);
      this.busy = false;
      if (r.code === 0) return closeOverlay(props().overlay);
      this.error = r.error || `herdr-helm failed (exit ${r.code})`;
      cx.notify();
    });
  }

  key(e, cx) {
    if (e.key === "escape") return this.close();
    if (this.mode === "new" && e.key === "enter" && e.modifiers?.platform) {
      cx.stop_propagation();
      return this.create(cx);
    }
    if (this.mode === "new" && e.key === "tab") {
      const order = ["name", ...(this.other ? [] : ["filter"]), ...(this.agent === "agent" ? ["goal"] : [])];
      const next = order[(order.indexOf(this.focused) + (e.modifiers?.shift ? order.length - 1 : 1)) % order.length];
      cx.stop_propagation();
      return this[next].focus();
    }
    if (e.key === "enter" && this.mode !== "new") {
      cx.stop_propagation();
      this.end(cx);
    }
  }

  // The look of Herdr's confirm dialog: the title with Esc, the content, then Cancel and the main
  // button. `main`: { label, ready, danger, act }; `status`: a line beside the buttons.
  frame(t, title, children, main, status) {
    const button = (id, label, bg, fg, border, enabled, act) => h_flex().id(id).h(32).px(14).flex_none().items_center().rounded(8)
      .border_1().border_color(border).bg(bg).text_color(fg)
      .when(enabled, (el) => el.cursor_pointer().hover((st) => st.opacity(0.88)).on_click((_e, cx) => act(cx)))
      .when(!enabled, (el) => el.opacity(0.45))
      .child(label);
    const mainBg = main.danger ? t.blocked : t.solid;
    return v_flex()
      .id("task")
      .track_focus(this.handle)
      .on_key_down((e, cx) => this.key(e, cx))
      .size_full()
      .p(18)
      .gap(10)
      .bg(t.popover)
      .text_size(13)
      .child(h_flex().items_center().child(div().flex_1().text_size(15).font_semibold().text_color(t.foreground).child(title))
        .child(div().text_size(11).text_color(t.dim).child("Esc")))
      .children(children)
      .when(Boolean(this.error), (el) => el.child(div().text_color(t.blocked).child(this.error)))
      .child(h_flex().gap(8).items_center().text_size(12).pt(4)
        .child(div().flex_1().min_w(0).truncate().text_size(11).text_color(t.dim).child(status ?? ""))
        .child(button("cancel", "Cancel", t.raised, t.foreground, t.input, !this.busy, () => this.close()))
        .child(button("confirm", main.label, mainBg, main.danger ? t.background : t.solid_foreground, mainBg, main.ready && !this.busy, (cx) => main.act(cx))));
  }

  input(t, state, key) {
    return Input.new(state).h(32).px(10).items_center().rounded(6).border(1)
      .border_color(this.focused === key ? t.accent : t.input).bg(t.background).text_color(t.foreground);
  }

  chip(t, id, text, on, click) {
    return div().id(id).px(8).h(22).flex().items_center().rounded(6).cursor_pointer().text_size(12)
      .when(on, (el) => el.bg(t.selected).text_color(t.foreground)).when(!on, (el) => el.text_color(t.muted).hover((s) => s.bg(t.hover)))
      .on_click(click).child(text);
  }

  renderNew(t) {
    const ps = this.projects;
    const p = this.project;
    const label = (text) => div().text_size(11).text_color(t.dim).child(text);
    const parts = [];
    if (ps === undefined) parts.push(div().text_color(t.muted).child("…"));
    else if (ps.length === 0) parts.push(div().text_color(t.muted).child("No projects. Add one in Helm Settings."));
    else {
      // Always shown, also with one project: it says where the space goes.
      parts.push(h_flex().gap(6).items_center().child(div().w(52).flex_none().text_size(11).text_color(t.dim).child("Project"))
        .children(ps.map((x) => this.chip(t, `project-${x.name}`, x.name, !this.other && x === p, (_e, cx) => { this.choose(x, cx); cx.notify(); })))
        .child(this.chip(t, "project-other", "Other", this.other, (_e, cx) => { this.other = true; this.error = ""; cx.notify(); })));
      parts.push(this.input(t, this.name, "name"));
      const title = this.name.value().trim();
      if (this.other) parts.push(...this.plainParts(t, label));
      else parts.push(label(title ? `Name and branch: ${slugify(title)}` : "The name and the branch come from this line."));
    }
    if (ps !== undefined && ps.length > 0 && !this.other) {
      const query = this.filter.value().trim().toLowerCase();
      const shown = this.order.filter((r) => r.toLowerCase().includes(query));
      const all = shown.length > 0 && shown.every((r) => this.picked.has(r));
      const set = (on, cx) => { for (const r of shown) on ? this.picked.add(r) : this.picked.delete(r); this.error = ""; cx.notify(); };
      parts.push(h_flex().gap(8).items_center().mt(4)
        .child(label(`Repos · ${this.picked.size} of ${this.order.length}`)).child(div().flex_1())
        .child(this.chip(t, "repos-all", "All", all, (_e, cx) => set(true, cx)))
        .child(this.chip(t, "repos-none", "None", false, (_e, cx) => set(false, cx))));
      parts.push(Input.new(this.filter).h(28).px(10).items_center().rounded(6).border(1).text_size(12)
        .border_color(this.focused === "filter" ? t.accent : t.input).bg(t.background).text_color(t.foreground));
      parts.push(div().relative().flex_1().min_h(0)
        .child(v_flex().id("task-repos").size_full().overflow_y_scroll()
          .when(shown.length === 0, (el) => el.child(div().p(8).text_color(t.dim).child("No repo matches.")))
          .children(shown.map((r) => {
            const on = this.picked.has(r), info = this.info[r];
            // A ticked repo shows the base its new branch starts from. Else the branch it is on.
            const tail = on ? (info?.base ? `from ${info.base}` : "") : (info?.branch ?? "");
            return h_flex().id(`repo-${r}`).h(28).flex_none().px(8).gap(9).items_center().rounded(6).cursor_pointer().hover((s) => s.bg(t.hover))
              .on_click((_e, cx) => { on ? this.picked.delete(r) : this.picked.add(r); this.error = ""; cx.notify(); })
              .child(checkBox(t, on))
              .child(div().flex_1().min_w(0).truncate().text_color(on ? t.foreground : t.muted).child(r))
              .when((p?.defaults ?? []).includes(r), (el) => el.child(label("default")))
              .child(div().max_w(170).flex_none().truncate().text_size(11).text_color(on ? t.muted : t.dim).child(tail));
          })))
        .child(Scrollbar.vertical("task-repos").absolute().inset_0()));
    }
    if (ps !== undefined && ps.length > 0) {
      parts.push(h_flex().gap(8).items_center().mt(4)
        .child(label("First tab")).child(div().flex_1())
        .when(Boolean(AGENT()), (el) => el.child(this.chip(t, "agent-agent", AGENT(), this.agent === "agent", (_e, cx) => { this.agent = "agent"; cx.notify(); })))
        .child(this.chip(t, "agent-shell", "Shell", this.agent === "shell", (_e, cx) => {
          this.agent = "shell";
          if (this.focused === "goal") this.name.focus();
          cx.notify();
        })));
      if (this.agent === "agent") parts.push(Textarea.new(this.goal).px(10).py(6).rounded(6).border(1)
        .border_color(this.focused === "goal" ? t.accent : t.input).bg(t.background).text_color(t.foreground));
    }
    const status = this.busy ? (this.progress || "Fetching and checking out the repos…") : "↵ or ⌘↵ create · tab next field";
    return this.frame(t, this.other ? "New space under Other" : `New space${p ? ` in ${p.name}` : ""}`, parts,
      { label: this.busy ? "Creating…" : "Create", ready: Boolean(this.project || this.other), act: (cx) => this.create(cx) }, status);
  }

  // Other: the folder of the plain space. The known folders as rows, and one you choose.
  plainParts(t, label) {
    const shown = [...this.folders.slice(0, 8)];
    if (this.folder && !shown.some((f) => f.path === this.folder)) shown.unshift({ path: this.folder });
    const row = (f) => {
      const on = f.path === this.folder;
      return h_flex().id(`folder-${f.path}`).h(28).flex_none().px(8).gap(9).items_center().rounded(6).cursor_pointer()
        .bg(on ? t.selected : t.popover).hover((s) => s.bg(on ? t.selected : t.hover))
        .on_click((_e, cx) => { this.folder = f.path; this.error = ""; cx.notify(); })
        .child(div().flex_none().text_color(on ? t.foreground : t.muted).child(f.path.split("/").pop() || f.path))
        .child(div().flex_1().min_w(0).truncate().text_size(11).text_color(t.dim).child(shortHome(f.path)));
    };
    return [
      label("A plain space in a folder: no checkouts, no branch."),
      h_flex().gap(8).items_center().mt(4).child(label("Folder")).child(div().flex_1())
        .child(this.chip(t, "folder-choose", "Choose folder…", false, (_e, cx) => {
          cx.spawn(async (cx) => {
            const path = await pickFolder({ prompt: "Use this folder" });
            if (path) { this.folder = path; this.error = ""; }
            cx.notify();
          });
        })),
      div().relative().flex_1().min_h(0)
        .child(v_flex().id("task-folders").size_full().overflow_y_scroll()
          .when(shown.length === 0, (el) => el.child(div().p(8).text_color(t.dim).child("No folders yet. Choose one.")))
          .children(shown.map(row)))
        .child(Scrollbar.vertical("task-folders").absolute().inset_0()),
    ];
  }

  render() {
    const t = theme();
    if (this.mode === "new") return this.renderNew(t);
    const space = props().space;
    const c = this.check;
    // The space's folder, from the project spaces that helm-jobs reads (the tasks folder may
    // be anywhere).
    const where = shortHome((shared("helm.spaces") ?? []).find((x) => x.name === space)?.path) || "its folder";
    const line = (x) => div().text_color(t.blocked).child(`${x.checkout ? `${x.checkout}: ` : ""}${x.what}`);
    let parts;
    if (this.mode === "complete") {
      parts = c === undefined
        ? [div().text_color(t.muted).child("Checking the checkouts…")]
        : c.ok
          ? [div().text_color(t.muted).child(`Everything is committed and pushed. This closes the space, removes its checkouts and ${where}, and keeps the branches.`)]
          : [div().text_color(t.muted).child("Not ready. Commit and push, and move the files that are not in a checkout:"), ...(c.problems ?? []).map(line)];

    } else {
      const lost = c?.losses ?? [];
      parts = c === undefined
        ? [div().text_color(t.muted).child("Checking what would be lost…")]
        : [div().text_color(t.muted).child(`This closes the space and ends its agents, removes its checkouts and ${where}, and deletes the local branches that Helm made. Pushed branches stay on the remote.`),
           ...(lost.length ? [div().font_semibold().text_color(t.blocked).child("This is lost:"), ...lost.map(line)] : [div().text_color(t.muted).child("Nothing unsaved is lost.")])];

    }
    const complete = this.mode === "complete";
    const main = {
      label: this.busy ? (complete ? "Completing…" : "Deleting…") : complete ? "Complete" : "Delete",
      ready: Boolean(complete ? c?.ok : c?.losses),
      danger: !complete,
      act: (cx) => this.end(cx),
    };
    return this.frame(t, `${complete ? "Complete" : "Delete"} space ${space}?`, [...parts, div().flex_1()], main);
  }
}
