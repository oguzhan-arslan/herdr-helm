// The Helm projects, in the style of the settings screen: the projects on the
// left, the chosen one on the right in three sections (General, Repos, Agent setup), and
// the global settings, and the connectors. Every change goes through `herdr-helm project` or
// `herdr-helm settings`,
// the only writers of projects.json.
// props { project? }: the one to show first.

import { View, div, svg } from "gpui-kit";
import { v_flex, h_flex, InputState, Input, Scrollbar } from "gpui-base";
import { props, closeOverlay, theme, appHome, shared, setShared } from "herdr";
import { helm, can, keep } from "./helm.js";
import { checkBox } from "./ui.js";

const SETTINGS = [
  ["copy", "Copy", "files copied from the root checkout, comma separated"],
  ["link", "Link", "files linked to the root checkout, comma separated"],
  ["setup", "Setup", "shell command run in each new checkout"],
  ["base", "Base", "start of the space branch, e.g. origin/develop"],
];
const SECTIONS = [["general", "General"], ["repos", "Repos"], ["agent", "Agent setup"]];
const ICON = (name) => `mods/workspace-chrome/${name}.svg`;

const task = helm;
// The kinds of connectors, with their built-in names. Any other value is a program.
const KINDS = [
  ["code_host", "Code host", ["github"], "PRs, reviews, and the merge queue."],
  ["agent", "Agent", ["claude-code"], "Starts, resumes, and names the agents."],
  ["editor", "Editor", ["vscode"], "Opens files and diffs."],
  ["system", "System", ["macos", "linux"], "CPU and memory in the bar."],
];
// Opens a file in the editor (the editor connector). Show a link to it only with
// can("editor", "open").
const edit = (path) => helm(["open", path]);

const list = (text) => text.split(",").map((x) => x.trim()).filter(Boolean);

// "link 1 · setup": what a repo has set, in short.
const summary = (settings) => Object.entries(settings)
  .map(([k, v]) => (Array.isArray(v) ? `${k} ${v.length}` : k === "base" ? `base ${v}` : k)).join(" · ");

export default class Projects extends View {
  init(_props, cx) {
    this.names = undefined; // the project names, once read
    this.chosen = props()?.project ?? null; // a name, "+" for a new one, or "*" for the settings
    this.section = "general";
    this.project = null; // `herdr-helm project show` of the chosen one
    this.settings = null; // `herdr-helm settings`
    this.open = null; // the repo whose settings show
    this.fields = {}; // that repo's setting inputs
    this.confirmRemove = false;
    this.error = "";
    this.saved = "";
    this.busy = false;
    this.handle = cx.focus_handle();
    this.handle.focus();
    const field = (placeholder, submit) => {
      const input = InputState.new({ placeholder });
      if (submit) input.on("submit", (_e, cx) => submit(cx));
      return input;
    };
    this.root = field("~/code", (cx) => this.saveGeneral(cx));
    this.base = field("each repo's own default, e.g. origin/main", (cx) => this.saveGeneral(cx));
    this.claudeMd = field("the built-in template", (cx) => this.saveAgent(cx));
    this.filter = field("Filter repos");
    this.filter.on("change", (_e, cx) => cx.notify());
    this.newName = field("project name, e.g. work", (cx) => this.add(cx));
    this.newRoot = field("root folder, e.g. ~/code", (cx) => this.add(cx));
    this.global = {
      host: field("github.com", (cx) => this.saveSettings(cx)),
      org: field("the org of the PRs popup", (cx) => this.saveSettings(cx)),
      refresh: field("5", (cx) => this.saveSettings(cx)),
      tasks: field("~/herdr/tasks", (cx) => this.saveSettings(cx)),
      smart: field("the agent's default", (cx) => this.saveSettings(cx)),
    };
    // The connector of each kind: a built-in name, "none", or a program.
    this.choice = Object.fromEntries(KINDS.map(([kind, , names]) => [kind, field(`${names[0]} (built in), none, or a program`, (cx) => this.saveConnectors(cx))]));
    // The data read last shows at once. The reads below bring it up to date.
    const names = shared("helm.projects");
    if (Array.isArray(names) && names.length) {
      this.names = names.map((p) => p.name);
      const first = this.chosen && this.names.includes(this.chosen) ? this.chosen : this.names[0];
      if (shared(`helm.project.${first}`)) {
        this.chosen = first;
        this.fill(shared(`helm.project.${first}`));
      }
    }
    this.reload(cx, this.chosen);
  }

  // Reads the projects again (and tells the sidebar), then shows `name`.
  reload(cx, name) {
    cx.spawn(async (cx) => {
      const r = await task(["projects"]);
      if (r.ok) {
        this.names = r.data.map((p) => p.name);
        keep("helm.projects", r.data);
      } else {
        this.names = [];
        this.error = r.error;
      }
      const pick = name && (name === "+" || name === "*" || name === "@" || this.names.includes(name)) ? name : this.names[0] ?? "+";
      await this.show(pick);
      cx.notify();
    });
  }

  async show(name) {
    const again = name === this.chosen && this.project?.name === name;
    this.chosen = name;
    if (!again) {
      this.project = null;
      this.open = null;
      this.confirmRemove = false;
      this.saved = "";
    }
    if (name === "+") return;
    if (name === "*" || name === "@") return this.showSettings();
    if (!again && shared(`helm.project.${name}`)) this.fill(shared(`helm.project.${name}`));
    const r = await task(["project", "show", name]);
    if (!r.ok) {
      this.error = r.error;
      return;
    }
    // A second read of the same project leaves the fields alone: the user may type in them.
    if (again || this.project?.name === name) {
      this.project = r.data;
      keep(`helm.project.${name}`, r.data);
    } else this.fill(r.data);
  }

  fill(p) {
    keep(`helm.project.${p.name}`, p);
    this.project = p;
    this.root.set_value(p.root);
    this.base.set_value(p.base ?? "");
    this.claudeMd.set_value(p.claude_md ?? "");
  }

  async showSettings() {
    const r = await task(["settings"]);
    if (!r.ok) {
      this.error = r.error;
      return;
    }
    this.settings = r.data;
    const host = r.data.code_host_settings, agent = r.data.agent_settings;
    this.global.host.set_value(host.host ?? "");
    this.global.org.set_value(host.org ?? "");
    this.global.refresh.set_value(String(host.refresh_minutes ?? 5));
    this.global.tasks.set_value(r.data.tasks_root ?? "");
    this.global.smart.set_value(agent.smart_command ?? "");
    this.global.smart.set_placeholder?.(agent.smart_default ?? "the agent's default");
    for (const [kind] of KINDS) {
      const c = r.data.connectors?.[kind];
      this.choice[kind].set_value(typeof c === "string" ? c : c?.command ?? "");
    }
    if (r.data.chosen && JSON.stringify(r.data.chosen) !== JSON.stringify(shared("helm.connectors"))) {
      keep("helm.connectors", r.data.chosen);
      // Usage just turned on: helm-jobs reads it now, not at its next turn.
      if ((r.data.chosen.agent?.capabilities ?? []).includes("usage") && !shared("agent.usage")) setShared("agent.usage.refresh", Date.now());
    }
  }

  // The data read last shows at once. The rest shows when the read is done.
  choose(name, cx) {
    this.error = "";
    const done = this.show(name);
    cx.notify();
    cx.spawn(async (cx) => {
      await done;
      cx.notify();
    });
  }

  // Replaces some fields of the shown project; on success shows what was saved.
  update(change, cx) {
    if (this.busy || !this.project) return;
    this.busy = true;
    this.error = "";
    this.saved = "";
    const name = this.project.name;
    const open = this.open;
    cx.spawn(async (cx) => {
      const r = await task(["project", "update", name, "--data", JSON.stringify(change)]);
      this.busy = false;
      if (!r.ok) this.error = r.error;
      else {
        this.fill(r.data);
        this.open = open;
        this.saved = "Saved";
        const all = await task(["projects"]);
        if (all.ok) keep("helm.projects", all.data);
      }
      cx.notify();
    });
  }

  saveGeneral(cx) {
    const root = this.root.value().trim();
    const base = this.base.value().trim();
    const change = {};
    if (root && root !== this.project?.root) change.root = root;
    if (base !== (this.project?.base ?? "")) change.base = base;
    if (Object.keys(change).length) this.update(change, cx);
  }

  saveAgent(cx) {
    const path = this.claudeMd.value().trim();
    if (path !== (this.project?.claude_md ?? "")) this.update({ claude_md: path }, cx);
  }

  saveSettings(cx) {
    if (this.busy) return;
    const minutes = Number(this.global.refresh.value().trim() || "5");
    if (!Number.isInteger(minutes) || minutes < 1) {
      this.error = "The refresh time is a whole number of minutes, 1 or more.";
      return cx.notify();
    }
    const change = {
      tasks_root: this.global.tasks.value().trim(),
      code_host_settings: { host: this.global.host.value().trim(), org: this.global.org.value().trim(), refresh_minutes: minutes },
      agent_settings: { smart_command: this.global.smart.value().trim() },
    };
    this.saveChange(change, cx);
  }

  // Agent usage on or off (agent_settings.usage). It reads a sign-in of the agent, so it is
  // off until you turn it on.
  saveUsage(on, cx) {
    this.saveChange({ agent_settings: { usage: on } }, cx);
  }

  // The connector of each kind: a built-in name or "none" as it is, any other text as a
  // program. Empty: the default.
  saveConnectors(cx) {
    const connectors = {};
    for (const [kind, , names] of KINDS) {
      const v = this.choice[kind].value().trim();
      connectors[kind] = !v ? null : v === "none" || names.includes(v) ? v : { command: v };
    }
    this.saveChange({ connectors }, cx);
  }

  saveChange(change, cx) {
    if (this.busy) return;
    this.busy = true;
    this.error = "";
    this.saved = "";
    cx.spawn(async (cx) => {
      const r = await task(["settings", "update", "--data", JSON.stringify(change)]);
      this.busy = false;
      if (!r.ok) this.error = r.error;
      else {
        await this.showSettings();
        this.saved = "Saved";
      }
      cx.notify();
    });
  }

  toggle(key, repo, cx) {
    const p = this.project;
    const on = p[key].includes(repo);
    const other = key === "defaults" ? "exclude" : "defaults";
    // Default and excluded rule each other out: ticking one unticks the other.
    this.update({ [key]: on ? p[key].filter((r) => r !== repo) : [...p[key], repo], [other]: p[other].filter((r) => r !== repo) }, cx);
  }

  expand(repo, cx) {
    if (this.open === repo) {
      this.open = null;
      return cx.notify();
    }
    this.open = repo;
    const settings = this.project.repos.find((r) => r.name === repo)?.settings ?? {};
    this.fields = Object.fromEntries(SETTINGS.map(([key, , hint]) => {
      const value = settings[key];
      const input = InputState.new({ placeholder: hint, value: Array.isArray(value) ? value.join(", ") : value ?? "" });
      input.on("submit", (_e, cx) => this.saveRepo(cx));
      return [key, input];
    }));
    cx.notify();
  }

  saveRepo(cx) {
    const repo = this.open;
    const settings = Object.fromEntries(SETTINGS.map(([key]) => {
      const text = this.fields[key].value().trim();
      return [key, key === "copy" || key === "link" ? list(text) : text];
    }));
    const repos = Object.fromEntries(this.project.repos.map((r) => [r.name, r.settings]));
    this.update({ repos: { ...repos, [repo]: settings } }, cx);
  }

  add(cx) {
    const name = this.newName.value().trim();
    const root = this.newRoot.value().trim();
    if (this.busy || !name || !root) {
      this.error = "Give a name and a root folder.";
      return cx.notify();
    }
    this.busy = true;
    this.error = "";
    cx.spawn(async (cx) => {
      const r = await task(["project", "add", name, "--root", root]);
      this.busy = false;
      if (!r.ok) {
        this.error = r.error;
        return cx.notify();
      }
      this.newName.set_value("");
      this.newRoot.set_value("");
      this.section = "general";
      this.reload(cx, name);
    });
  }

  remove(cx) {
    if (!this.confirmRemove) {
      this.confirmRemove = true;
      return cx.notify();
    }
    const name = this.project.name;
    this.busy = true;
    cx.spawn(async (cx) => {
      const r = await task(["project", "remove", name]);
      this.busy = false;
      if (!r.ok) this.error = r.error;
      this.reload(cx, null);
    });
  }

  key(e) {
    if (e.key === "escape" && !this.busy) closeOverlay(props().overlay);
  }

  input(t, state, h = 30) {
    return Input.new(state).h(h).px(10).items_center().rounded(6).border(1).border_color(t.input).bg(t.background).text_color(t.foreground);
  }

  button(t, id, text, act, primary = false) {
    return div().id(id).px(12).h(28).flex().flex_none().items_center().rounded(6).cursor_pointer()
      .when(primary, (el) => el.bg(t.accent).text_color(t.background))
      .when(!primary, (el) => el.text_color(t.muted).hover((s) => s.bg(t.hover)))
      .on_click((_e, cx) => act(cx)).child(text);
  }

  link(t, id, text, act) {
    return div().id(id).cursor_pointer().text_size(11.5).text_color(t.accent).on_click((_e, cx) => act(cx)).child(text);
  }

  box(t, id, on, label, act) {
    return h_flex().id(id).gap(6).items_center().cursor_pointer().text_size(11.5).text_color(on ? t.foreground : t.dim)
      .on_click((_e, cx) => { cx.stop_propagation(); act(cx); })
      .child(checkBox(t, on))
      .child(label);
  }

  // A row of a form: the label on the left, then the field, and a note below the field.
  row(t, label, field, note) {
    return v_flex().gap(4)
      .child(h_flex().gap(10).items_center()
        .child(div().w(110).flex_none().text_size(12).text_color(t.muted).child(label))
        .child(div().flex_1().min_w(0).child(field)))
      .when(Boolean(note), (el) => el.child(div().pl(120).text_size(11).text_color(t.dim).child(note)));
  }

  heading(t, title, sub) {
    return v_flex().gap(2).pb(4)
      .child(div().text_size(16).font_semibold().text_color(t.foreground).child(title))
      .when(Boolean(sub), (el) => el.child(div().text_size(11.5).text_color(t.dim).child(sub)));
  }

  group(t, title) {
    return div().pt(8).text_size(11).font_semibold().text_color(t.dim).child(title);
  }

  nav(t) {
    const row = (id, label, icon, on, act) => h_flex().id(id).h(30).px(10).gap(9).items_center().rounded(7).cursor_pointer()
      .text_color(on ? t.foreground : t.muted).when(on, (el) => el.bg(t.selected)).when(!on, (el) => el.hover((s) => s.bg(t.hover)))
      .on_click(act).child(svg(ICON(icon)).size(14).flex_none()).child(div().truncate().child(label));
    return v_flex().w(200).flex_none().h_full().p(10).gap(2).bg(t.chrome).border_r(1).border_color(t.border)
      .child(div().px(10).pt(4).pb(10).font_semibold().text_color(t.foreground).child("Helm Settings"))
      .children((this.names ?? []).map((n) => row(`nav-${n}`, n, "folder", this.chosen === n, (_e, cx) => this.choose(n, cx))))
      .child(row("nav-add", "Add project", "plus", this.chosen === "+", (_e, cx) => this.choose("+", cx)))
      .child(div().flex_1())
      .child(row("nav-settings", "Settings", "settings", this.chosen === "*", (_e, cx) => this.choose("*", cx)))
      .child(row("nav-connectors", "Connectors", "server", this.chosen === "@", (_e, cx) => this.choose("@", cx)))
      .when(can("editor", "open"), (el) => el.child(row("nav-json", "Open projects.json", "file-text", false, () => edit(`${appHome()}/projects.json`))));
  }

  addForm(t) {
    return v_flex().gap(12)
      .child(this.heading(t, "Add project", "A project is a folder of git repos. Spaces in it get a checkout of the repos you pick."))
      .child(this.row(t, "Name", this.input(t, this.newName), "Lowercase letters, digits, and dashes."))
      .child(this.row(t, "Root folder", this.input(t, this.newRoot), "The folder that holds the repos, one level down."))
      .child(h_flex().pl(120).child(this.button(t, "add-go", this.busy ? "Adding…" : "Add project", (cx) => this.add(cx), true)));
  }

  settingsForm(t) {
    if (!this.settings) return div().text_color(t.muted).child("…");
    const chosen = this.settings.chosen ?? {};
    const agent = this.settings.agent_settings;
    // The settings of a built-in connector show only while it is the chosen one. A program
    // reads its own "<kind>_settings" block: edit it in projects.json.
    const github = chosen.code_host?.name === "github", claude = chosen.agent?.name === "claude-code";
    return v_flex().gap(12)
      .child(this.heading(t, "Settings", "For all projects. Pick the connectors on the Connectors page."))
      .child(this.group(t, "SPACES"))
      .child(this.row(t, "Tasks folder", this.input(t, this.global.tasks), "New spaces go here. Spaces in the old folder stay there, and do not show."))
      .when(github, (el) => el
        .child(this.group(t, "GITHUB"))
        .child(this.row(t, "Host", this.input(t, this.global.host), "The host gh uses, e.g. github.com or your-company.ghe.com."))
        .child(this.row(t, "Organization", this.input(t, this.global.org), "The PRs popup shows the open PRs of this org."))
        .child(this.row(t, "Refresh", this.input(t, this.global.refresh), "Minutes between two reads of the PRs.")))
      .when(claude, (el) => el
        .child(this.group(t, "CLAUDE CODE"))
        .child(this.row(t, "Smart rename", this.input(t, this.global.smart),
          `Gets all the facts of a space on stdin and prints JSON. Empty uses ${agent.smart_default}, with no tools.`))
        .child(this.row(t, "Usage", this.box(t, "usage-on", agent.usage, "Show the usage limits in the bar", (cx) => this.saveUsage(!agent.usage, cx)),
          "Reads the Claude Code sign-in from the macOS keychain, and asks Anthropic for the limits of your plan. Nothing else leaves your machine.")))
      .child(h_flex().pl(120).child(this.button(t, "settings-save", "Save", (cx) => this.saveSettings(cx), true)));
  }

  connectorsForm(t) {
    if (!this.settings) return div().text_color(t.muted).child("…");
    const chosen = this.settings.chosen ?? {};
    const kind = ([id, label, names, about]) => {
      const c = chosen[id] ?? { name: "none", capabilities: [] };
      const caps = c.capabilities?.length ? c.capabilities.join(", ").replaceAll("_", " ") : "nothing: the parts of the screen that need it do not show";
      return v_flex().gap(4)
        .child(this.row(t, label, this.input(t, this.choice[id]), `${about} Now: ${c.name}, which can do ${caps}.`));
    };
    return v_flex().gap(12)
      .child(this.heading(t, "Connectors", "Helm reaches each outside tool through a connector. Write a built-in name, none to turn the kind off, or a program that speaks the JSON contract of docs/connectors.md. Empty uses the default."))
      .children(KINDS.map(kind))
      .child(h_flex().pl(120).child(this.button(t, "connectors-save", "Save", (cx) => this.saveConnectors(cx), true)));
  }

  general(t, p) {
    return v_flex().size_full().gap(12)
      .child(this.row(t, "Root folder", this.input(t, this.root), `${p.repos.length} repos · ${p.defaults.length} default · ${p.exclude.length} excluded`))
      .child(this.row(t, "Base branch", this.input(t, this.base),
        "New space branches start here, in each repo that has it. The base of a repo comes first."))
      .child(h_flex().pl(120).child(this.button(t, "general-save", "Save", (cx) => this.saveGeneral(cx), true)))
      .child(div().flex_1())
      .child(h_flex().pt(10).border_t(1).border_color(t.border).items_center()
        .child(div().flex_1().text_size(11).text_color(t.dim).child("Removing keeps its spaces: they move to Other."))
        .child(div().id("remove").px(12).h(28).flex().items_center().rounded(6).cursor_pointer().text_color(t.blocked).hover((s) => s.bg(t.hover))
          .on_click((_e, cx) => this.remove(cx)).child(this.confirmRemove ? "Click again to remove" : "Remove project")));
  }

  repoRow(t, r) {
    const p = this.project;
    const isDefault = p.defaults.includes(r.name);
    const excluded = p.exclude.includes(r.name);
    const open = this.open === r.name;
    return v_flex().flex_none().rounded(7).when(open, (el) => el.bg(t.raised))
      .child(h_flex().id(`repo-${r.name}`).h(42).px(8).gap(12).items_center().rounded(7).cursor_pointer().hover((s) => s.bg(t.hover))
        .on_click((_e, cx) => this.expand(r.name, cx))
        .child(svg(ICON(`chevron-${open ? "down" : "right"}`)).size(11).flex_none().text_color(t.dim))
        .child(v_flex().flex_1().min_w(0)
          .child(div().truncate().text_color(excluded ? t.dim : t.foreground).child(r.name))
          .child(div().truncate().text_size(11).text_color(t.dim)
            .child([r.host, r.branch && `on ${r.branch}`, summary(r.settings)].filter(Boolean).join(" · "))))
        .child(this.box(t, `def-${r.name}`, isDefault, "default", (cx) => this.toggle("defaults", r.name, cx)))
        .child(this.box(t, `exc-${r.name}`, excluded, "excluded", (cx) => this.toggle("exclude", r.name, cx))))
      .when(open, (el) => el.child(v_flex().px(10).pb(10).pt(2).gap(6)
        .child(div().text_size(11).text_color(t.dim).child(`New branches start from ${r.base || "the default branch of the repo"}.`))
        .children(SETTINGS.map(([key, label]) => h_flex().gap(10).items_center()
          .child(div().w(50).flex_none().text_size(11.5).text_color(t.muted).child(label))
          .child(div().flex_1().child(this.input(t, this.fields[key])))))
        .child(h_flex().justify_end().child(this.button(t, `save-${r.name}`, "Save", (cx) => this.saveRepo(cx), true)))));
  }

  repos(t, p) {
    const query = this.filter.value().trim().toLowerCase();
    const shown = p.repos.filter((r) => r.name.toLowerCase().includes(query));
    return v_flex().size_full().gap(8)
      .child(h_flex().gap(10).items_center()
        .child(div().flex_1().child(this.input(t, this.filter, 28)))
        .child(div().text_size(11).text_color(t.dim).child(`${shown.length} of ${p.repos.length}`)))
      .child(div().text_size(11).text_color(t.dim).child("A new space ticks the default repos and hides the excluded ones. Click a repo for its settings."))
      .child(div().relative().flex_1().min_h(0)
        .child(v_flex().id("repos").size_full().overflow_y_scroll().gap(1)
          .when(shown.length === 0, (el) => el.child(div().p(8).text_color(t.dim).child("No repo matches.")))
          .children(shown.map((r) => this.repoRow(t, r))))
        .child(Scrollbar.vertical("repos").absolute().inset_0()));
  }

  agent(t, p) {
    return v_flex().gap(12)
      .child(this.row(t, "CLAUDE.md", this.input(t, this.claudeMd),
        "A template file for the CLAUDE.md of each new space. Empty uses the built-in one."))
      .child(h_flex().pl(120).gap(14).items_center()
        .child(this.button(t, "agent-save", "Save", (cx) => this.saveAgent(cx), true))
        .when(can("editor", "open"), (el) => el.child(this.link(t, "agent-open", "Open the template in the editor", () => edit(p.claude_md_path)))))
      .child(this.group(t, "SKILLS"))
      .child(div().text_size(11).text_color(t.dim)
        .child(`Each space gets ${p.skills_builtin.join(", ")}. The space and its checkouts share their skills.`));
  }

  details(t) {
    const p = this.project;
    if (!p) return div().text_color(t.muted).child("…");
    const tab = (id, label) => div().id(`section-${id}`).px(2).pb(8).cursor_pointer().border_b(2)
      .border_color(this.section === id ? t.accent : t.popover)
      .text_color(this.section === id ? t.foreground : t.muted)
      .on_click((_e, cx) => { this.section = id; this.saved = ""; cx.notify(); }).child(label);
    return v_flex().size_full().gap(14)
      .child(this.heading(t, p.name, p.root_path))
      .child(h_flex().gap(20).border_b(1).border_color(t.border).children(SECTIONS.map(([id, label]) => tab(id, label))))
      .child(v_flex().flex_1().min_h(0)
        .child(this.section === "general" ? this.general(t, p) : this.section === "repos" ? this.repos(t, p) : this.agent(t, p)));
  }

  render() {
    const t = theme();
    const page = this.names === undefined ? div().text_color(t.muted).child("…")
      : this.chosen === "+" ? this.addForm(t) : this.chosen === "*" ? this.settingsForm(t) : this.chosen === "@" ? this.connectorsForm(t) : this.details(t);
    return h_flex().id("projects").track_focus(this.handle).on_key_down((e) => this.key(e)).size_full().bg(t.popover).text_size(13)
      .child(this.nav(t))
      .child(v_flex().flex_1().min_w(0).h_full().p(20).gap(10)
        // Settings and Add project scroll as one page; a project's tabs scroll their own lists.
        .child(this.chosen === "+" || this.chosen === "*" || this.chosen === "@"
          ? div().relative().flex_1().min_h(0)
            .child(v_flex().id("page-scroll").size_full().overflow_y_scroll().pr(12).child(page))
            .child(Scrollbar.vertical("page-scroll").absolute().inset_0())
          : v_flex().flex_1().min_h(0).child(page))
        .when(Boolean(this.error), (el) => el.child(div().text_color(t.blocked).child(this.error)))
        .child(h_flex().justify_end().text_size(11).text_color(t.dim).child(this.busy ? "saving…" : this.saved ? `${this.saved} · esc close` : "esc close")));
  }
}
