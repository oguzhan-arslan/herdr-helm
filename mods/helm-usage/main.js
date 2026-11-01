// The usage popups of helm. props { system: true }: the apps that weigh most on the machine
// (from the CPU and memory in the bar), from `herdr-helm system --apps`. Else: the agent's
// usage limits (helm-jobs reads them into shared "agent.usage"), with reset times and pace.
import { View, div, svg } from "gpui-kit";
import { h_flex, v_flex } from "gpui-base";
import { theme, shared, setShared, props, window } from "herdr";
import { helm, agentIcon } from "./helm.js";
import { agoText } from "./ui.js";

// The agent connector's name, as the popup shows it.
const AGENT = () => shared("helm.connectors")?.agent?.label ?? "Agent";

// Calm until it matters: green, amber from 60%, orange from 80%, red from 92%.
function level(pct, t) {
  const hex = (t.chrome ?? "#000000").replace("#", "");
  const light = [0, 2, 4].reduce((s, i) => s + parseInt(hex.slice(i, i + 2), 16), 0) > 3 * 128;
  const scale = light ? ["#1f9d63", "#c98a0b", "#dd6b20", "#d64545"] : ["#5ed49a", "#f2c14e", "#f59152", "#f2665c"];
  return scale[pct >= 92 ? 3 : pct >= 80 ? 2 : pct >= 60 ? 1 : 0];
}

// Share of the window already gone, 0..1; null without a reset time.
function elapsed(l, now) {
  if (!l.resetsAt) return null;
  const left = (Date.parse(l.resetsAt) - now) / 1000;
  return Math.min(1, Math.max(0, 1 - left / l.seconds));
}

function until(iso, now) {
  const min = Math.max(0, Math.round((Date.parse(iso) - now) / 60000));
  if (min < 60) return `${min}m`;
  const h = Math.floor(min / 60);
  return h < 24 ? `${h}h ${min % 60}m` : `${Math.floor(h / 24)}d ${h % 24}h`;
}

function pace(l, now) {
  const e = elapsed(l, now);
  if (e === null || e < 0.03) return null;
  return Math.round(l.percent - e * 100);
}


function bar(l, width, height, now, t) {
  const e = elapsed(l, now), pct = Math.min(100, Math.max(0, l.percent));
  return div().relative().w(width).h(height).flex_none().rounded(height / 2).bg(t.border)
    .child(div().absolute().left(0).top(0).h(height).w(Math.max(pct > 0 ? height : 0, width * pct / 100)).rounded(height / 2).bg(level(pct, t)))
    .when(e !== null, el => el.child(div().absolute().top(-2).h(height + 4).w(1).left(Math.round(width * e)).bg(t.foreground).opacity(0.45)));
}

export default class Usage extends View {
  init(_props, cx) {
    if (props()?.system) this.initApps(cx);
  }

  initApps(cx) {
    this.apps = null;
    this.readApps(cx);
    cx.timer.every(5000, (cx) => this.readApps(cx));
  }

  readApps(cx) {
    if (this.appsLoading) return;
    this.appsLoading = true;
    cx.spawn(async (cx) => {
      try {
        const r = await helm(["system", "--apps"]);
        if (r.data) this.apps = r.data.apps ?? [];
      } catch {}
      finally { this.appsLoading = false; cx.notify(); }
    });
  }

  appsMenu(t) {
    const apps = this.apps ?? [];
    const top = Math.max(1, ...apps.map((a) => a.impact));
    const gb = (kb) => (kb >= 1048576 ? `${(kb / 1048576).toFixed(1)} GB` : `${Math.round(kb / 1024)} MB`);
    const head = (text, w) => div().w(w).flex_none().text_right().text_size(10.5).text_color(t.dim).child(text);
    return v_flex().id("apps-menu").w_full().p(6).bg(t.popover).text_size(12.5)
      .child(h_flex().px(8).pt(2).pb(6).gap(8).items_center()
        .child(div().flex_1().text_size(11).text_color(t.dim).child("Top apps by CPU + memory \u00b7 every 5 seconds"))
        .child(head("CPU", 52)).child(head("Memory", 64)).child(head("Impact", 70)))
      .when(!this.apps, (el) => el.child(div().px(8).py(6).text_color(t.dim).child("Reading\u2026")))
      .children(apps.map((a) => h_flex().id(`app-${a.name}`).h(26).flex_none().px(8).gap(8).items_center().rounded(6)
        
        .child(div().flex_1().min_w(0).truncate().text_color(t.foreground).child(a.name))
        .child(div().w(52).flex_none().text_right().text_size(11.5).text_color(a.cpu >= 100 ? level(80, t) : t.muted).child(`${Math.round(a.cpu)}%`))
        .child(div().w(64).flex_none().text_right().text_size(11.5).text_color(t.muted).child(gb(a.kb)))
        // Length against the top app; color by the impact itself (red only when it is high).
        .child(h_flex().w(70).flex_none().justify_end().child(div().relative().w(60).h(4).rounded(2).bg(t.border)
          .child(div().absolute().left(0).top(0).h(4).rounded(2).w(Math.max(2, 60 * a.impact / top)).bg(level(a.impact, t)))))));
  }

  menu(t) {
    const data = shared("agent.usage"), error = shared("agent.usage.error"), now = Date.now();
    return v_flex().id("usage-menu").w_full().p(5).bg(t.popover).text_size(12.5)
      .child(h_flex().px(8).pt(4).pb(2).items_center()
        .child(div().flex_1().text_size(11).text_color(t.dim).child(data ? `${AGENT()} usage · updated ${agoText(data.at)}` : `${AGENT()} usage`))
        .child(div().id("usage-refresh").px(6).rounded(5).cursor_pointer().text_size(11).text_color(t.muted).hover(s => s.bg(t.hover))
          .on_click(() => setShared("agent.usage.refresh", Date.now())).child("Refresh")))
      .when(Boolean(error), el => el.child(div().px(8).py(4).text_size(11).text_color(level(95, t)).child(error)))
      .when(Boolean(data), el => el.child(v_flex().px(8).py(4).gap(4)
        .child(h_flex().gap(6).items_center()
          .child(svg(agentIcon(t).path).size(13).flex_none().text_color(agentIcon(t).color))
          .child(div().font_semibold().text_color(t.foreground).child(AGENT()))
          .child(div().text_size(10.5).text_color(t.dim).child(data.plan)))
        .children(data.limits.map(l => {
          const p = pace(l, now);
          return v_flex().gap(3)
            .child(h_flex().gap(8).items_center()
              .child(div().w(62).flex_none().truncate().text_size(11.5).text_color(t.muted).child(l.label))
              .child(bar(l, 120, 5, now, t))
              .child(div().flex_1().text_right().text_size(11.5).text_color(l.percent >= 60 ? level(l.percent, t) : t.foreground).child(`${Math.round(l.percent)}%`)))
            .child(div().pl(70).text_size(10.5).text_color(t.dim).child([
              l.resetsAt && Date.parse(l.resetsAt) > now ? `resets in ${until(l.resetsAt, now)}` : null,
              p === null ? null : Math.abs(p) < 5 ? "on pace" : p > 0 ? `${p}% ahead of pace` : `${-p}% under pace`,
            ].filter(Boolean).join(" · ")));
        }))));
  }

  render() {
    const t = theme();
    return props()?.system ? this.appsMenu(t) : this.menu(t);
  }
}
