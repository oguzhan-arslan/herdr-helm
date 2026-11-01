// The background work of the helm layout, in its 0-px "jobs-run" block (Herdr starts a mod
// only once its block shows, so the block is shown, not hidden). No view. It keeps these shared:
//   helm.connectors                the chosen connectors and what they can do
//   helm.projects, helm.spaces     the projects and the project spaces
//   smart.running                  the spaces Smart rename works on (asked in "smart.request")
//   agent.usage                    the agent's usage limits (asked again in "agent.usage.refresh")
//   github.cache                   the repos and PRs of each shown space ("github.refresh")
//   prs.data                       the org's open PRs ("prs.refresh")
// And it saves which agent session runs in which terminal, to resume them after a restart.
//   prs.loading, prs.error, agent.usage.error   the state of those reads
// A shared write makes every mod render again: each job writes only what changed (keep).

import { View, div } from "gpui-kit";
import { spaces, home, shared, setShared, notify } from "herdr";
import { helm, helmSlow, can, keep } from "./helm.js";
import { awake, place } from "./ui.js";

const USAGE_EVERY = 5 * 60000, GITHUB_EVERY = 2 * 60000, GITHUB_FRESH = 30000, CONNECTORS_EVERY = 60000;
// herdr-helm asks GitHub at most every few hours and keeps the answer, so an hourly look is cheap.
const UPDATE_EVERY = 3600000;

// A new read `b` that brings nothing new over the kept read `a`: only its time "at" differs.
// The popups show "updated Xm ago" from "at", so a kept read older than 10 minutes is new too.
const STALE = 10 * 60000;
const sameRead = (a, b) => Boolean(a && b) && Date.now() - (a.at ?? 0) < STALE
  && JSON.stringify({ ...a, at: 0 }) === JSON.stringify({ ...b, at: 0 });

export default class Jobs extends View {
  init(_props, cx) {
    this.busy = new Set(); // the jobs that run now
    this.asked = {}; // request key -> the last request answered
    this.readAt = {}; // space -> when its github data was read last
    for (const key of ["smart.request", "agent.usage.refresh", "github.refresh", "prs.refresh"]) this.asked[key] = shared(key)?.at ?? shared(key) ?? 0;

    this.connectors(cx);
    this.tasks(cx);
    this.sessions(cx, "resume");
    this.usage(cx);
    this.prs(cx);
    cx.timer.every(20000, (cx) => awake() && this.tasks(cx));
    cx.timer.every(CONNECTORS_EVERY, (cx) => awake() && this.connectors(cx));
    cx.timer.after(30000, (cx) => this.update(cx));
    cx.timer.every(UPDATE_EVERY, (cx) => awake() && this.update(cx));
    let tick = 0;
    // Kept while away (a shutdown can come then), but every 2 minutes instead of 30 s.
    cx.timer.every(30000, (cx) => { if (awake() || ++tick % 4 === 0) this.sessions(cx, "save"); });
    cx.timer.every(USAGE_EVERY, (cx) => awake() && this.usage(cx));
    cx.timer.every(GITHUB_EVERY, (cx) => awake() && this.github(cx, true));
    cx.timer.every(60000, (cx) => awake() && this.prsDue() && this.prs(cx));

    let wasAwake = awake();
    let spaceIds = spaces().map((s) => s.id).join();
    let shown = null;
    // Requests, a switch of space, new spaces, and a return to Herdr: looked at each 400 ms.
    cx.timer.every(400, (cx) => {
      const ask = (key) => {
        const v = shared(key);
        const at = v?.at ?? v ?? 0;
        if (at === this.asked[key]) return null;
        this.asked[key] = at;
        return v;
      };
      const smart = ask("smart.request");
      if (smart?.id) this.smart(smart.id, cx);
      if (ask("agent.usage.refresh")) this.usage(cx, true);
      if (ask("github.refresh")) this.github(cx, true, true);
      if (ask("prs.refresh")) this.prs(cx);
      const ids = spaces().map((s) => s.id).join();
      if (ids !== spaceIds) { spaceIds = ids; this.tasks(cx); }
      const active = spaces().find((s) => s.active)?.id ?? null;
      if (active !== shown) { shown = active; this.github(cx, false); }
      const now = awake();
      if (now && !wasAwake) {
        this.connectors(cx);
        this.tasks(cx);
        if (Date.now() - (this.usageAt ?? 0) > USAGE_EVERY) this.usage(cx);
        this.github(cx, false);
        if (this.prsDue()) this.prs(cx);
      }
      wasAwake = now;
    });
  }

  // One run of a job at a time.
  once(name, cx, body) {
    if (this.busy.has(name)) return;
    this.busy.add(name);
    cx.spawn(async (cx) => {
      try { await body(cx); } catch {}
      finally { this.busy.delete(name); }
    });
  }

  // The chosen connector of each kind and what it can do: the UI hides what is not there.
  connectors(cx) {
    this.once("connectors", cx, async () => {
      const r = await helm(["connectors"]);
      if (r.data) keep("helm.connectors", r.data);
    });
  }

  // A newer Helm release: shared "helm.update" ({current, latest, available, url}). The ⋯ button
  // shows a dot, and the ⋯ menu an Update row. Nothing installs until the person clicks it.
  update(cx) {
    this.once("update", cx, async () => {
      const r = await helm(["update", "--check"]);
      if (r.data) keep("helm.update", r.data);
    });
  }

  // The projects and the project spaces.
  tasks(cx) {
    this.once("tasks", cx, async () => {
      const [projects, list] = await Promise.all([helm(["projects"]), helm(["spaces"])]);
      if (projects.data) keep("helm.projects", projects.data);
      if (list.data) keep("helm.spaces", list.data);
    });
  }

  // Agent tabs that a shutdown ended come back. At start, `herdr-helm sessions resume` types
  // the agent's resume command into each recovered shell. Every 30 s, `save` records which
  // session runs in which terminal. Both only act on what changed, so a reload is safe.
  sessions(cx, action) {
    if (!can("agent", "sessions")) return;
    this.once(`sessions-${action}`, cx, async () => {
      const r = await helm(["sessions", action]);
      const resumed = action === "resume" ? r.data?.resumed ?? [] : [];
      if (resumed.length) notify({ title: `Resumed ${resumed.length} agent ${resumed.length === 1 ? "session" : "sessions"}`,
        body: resumed.map((x) => x.cwd.split("/").pop()).join(", "), level: "success" });
    });
  }

  // Smart rename: the configured agent reads all of the space's work and names it.
  smart(id, cx) {
    this.once(`smart-${id}`, cx, async () => {
      setShared("smart.running", { ...(shared("smart.running") ?? {}), [id]: Date.now() });
      const r = await helm(["rename", "--herdr-space", id]);
      const d = r.ok ? r.data : null;
      const running = { ...(shared("smart.running") ?? {}) };
      delete running[id];
      setShared("smart.running", running);
      notify(d
        ? { title: d.title, body: d.summary || "Renamed.", level: "info" }
        : { title: "Smart rename failed", body: r.error, level: "warning" });
      this.tasks(cx);
    });
  }

  // The agent's usage limits, when its connector can read them (agent_settings.usage). `asked`:
  // the person asked for it, so the popup shows the new time also when nothing else changed.
  usage(cx, asked = false) {
    if (!can("agent", "usage")) return;
    this.once("usage", cx, async () => {
      const r = await helm(["usage"]);
      this.usageAt = Date.now();
      if (r.data) {
        if (asked || !sameRead(shared("agent.usage"), r.data)) setShared("agent.usage", r.data);
        keep("agent.usage.error", null);
      } else keep("agent.usage.error", r.error);
    });
  }

  // The shown space's repos and their PRs, from `herdr-helm github`: a project space's
  // checkouts, else the repos of the folder its first terminal is in. A switch reads again
  // only when the kept data is not fresh.
  // `asked`: the person asked for it (see usage).
  github(cx, force, asked = false) {
    const space = spaces().find((s) => s.active);
    if (!space || !can("code_host", "branch_prs")) return;
    const kept = (shared("github.cache") ?? {})[space.id];
    if (!force && kept && Date.now() - (this.readAt[space.id] ?? kept.at) < GITHUB_FRESH) return;
    this.once(`github-${space.id}`, cx, async () => {
      const r = await helm(["github", "--herdr-space", space.id, "--folder", place(space) || home()]);
      this.readAt[space.id] = Date.now();
      // A failed read keeps what the space showed, with the error beside it.
      const before = (shared("github.cache") ?? {})[space.id];
      const data = r.data ? { ...r.data, error: null } : before ? { ...before, error: r.error } : { at: Date.now(), repos: [], error: r.error };
      const ids = new Set(spaces().map((s) => s.id));
      const cache = Object.fromEntries(Object.entries(shared("github.cache") ?? {}).filter(([k]) => ids.has(k)));
      keep("github.cache", { ...cache, [space.id]: !asked && sameRead(before, data) ? before : data });
    });
  }

  prsDue() {
    const data = shared("prs.data");
    return !data || Date.now() - data.at >= (data.refresh_minutes ?? 5) * 60000 - 30000;
  }

  prs(cx) {
    if (!can("code_host", "org_prs")) return;
    this.once("prs", cx, async () => {
      keep("prs.loading", true);
      try {
        const r = await helmSlow(["prs"]);
        if (r.ok && r.data) { keep("prs.data", r.data); keep("prs.error", null); }
        else keep("prs.error", r.error || "herdr-helm prs failed");
      } finally { keep("prs.loading", false); }
    });
  }

  render() {
    return div();
  }
}
