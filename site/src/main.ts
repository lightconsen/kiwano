// ── 页面交互逻辑。英文源在模板 index.html 里;这里只负责语言切换、复制、
//    进场动画、下载计数、策略示意连线与实时演示的懒加载。 ──
import { I18N } from "./i18n";

// 应用语言:en 还原模板内联文案,zh 从字典取(先缓存原文,来回切不丢)。
// 已带 data-en 的元素(如下载计数行的 {n} 模板)不缓存 innerHTML——它们
// 的模板写在属性里,初始 innerHTML 是空的,缓存会把它抹掉。
document.querySelectorAll<HTMLElement>("[data-i18n]").forEach((el) => {
  if (el.dataset.en === undefined) el.dataset.en = el.innerHTML;
});
// 属性型文案(aria-label/title 同 key):与文本同一字典,同一处切换
document.querySelectorAll("[data-i18n-aria]").forEach((el) => {
  el.dataset.enAria = el.getAttribute("aria-label") ?? "";
});
// 下载计数:site/data/hub-stats.json 由每周的 hub-stats 工作流刷新并随站点
// 部署,取到后填进下载节;取不到就保持隐藏——页面不显示一个它无法佐证的数字。
let dlCountValue: number | null = null;
function applyLang(lang: "en" | "zh") {
  document.documentElement.lang = lang === "zh" ? "zh-CN" : "en";
  document.title = lang === "zh" ? I18N.zh.title : "Kiwano — Local-first AI Provider Manager";
  document.querySelectorAll<HTMLElement>("[data-i18n]").forEach((el) => {
    if (lang === "zh") {
      const v = I18N.zh[el.dataset.i18n!];
      if (v !== undefined) el.innerHTML = v;
    } else {
      el.innerHTML = el.dataset.en!;
    }
  });
  // 计数行带着已取到的数字走一遍,否则切语言会把 {n} 原样写回去。
  const countEl = document.getElementById("dlCount");
  if (countEl && !countEl.hidden && dlCountValue !== null) {
    countEl.innerHTML =
      (lang === "zh" ? I18N.zh["dl.count"] : countEl.dataset.en!).replace("{n}", dlCountValue.toLocaleString("en-US"));
  }
  document.querySelectorAll<HTMLElement>("[data-i18n-aria]").forEach((el) => {
    const v = lang === "zh" ? I18N.zh[el.dataset.i18nAria!] : el.dataset.enAria;
    if (v !== undefined) {
      el.setAttribute("aria-label", v);
      el.title = v;
    }
  });
  // 按钮显示「可切换到的语言」的国旗(emoji 旗在 Windows 上不渲染,所以是内联 SVG)
  document.getElementById("langFlag")!.setAttribute("href", lang === "zh" ? "#i-flag-gb" : "#i-flag-cn");
  // 文档链接跟随语言:zh 文档在 /docs/zh-CN/(FAQ 答案里的链接在字典里自带 href)
  document.getElementById("docsLink")!.href = lang === "zh" ? "/docs/zh-CN/" : "/docs/";
  localStorage.setItem("kiwano.lang", lang);
}

// 恢复上次选择(默认英文,即模板原文)
const saved = localStorage.getItem("kiwano.lang");
if (saved === "zh") applyLang("zh");
document.getElementById("langBtn")!.addEventListener("click", () => {
  applyLang(document.documentElement.lang === "zh-CN" ? "en" : "zh");
});

// 复制按钮:icon 即状态——平时是 copy,落成 check 一拍半再换回。
// Clipboard API 需要安全上下文;file:// 与旧 webview 走 execCommand 兜底。
document.querySelectorAll<HTMLButtonElement>("[data-copy]").forEach((btn) => {
  const text = btn.dataset.copy!;
  const label = () =>
    document.documentElement.lang === "zh-CN" ? I18N.zh["srv.copied"] : "Copied";
  const restore = () =>
    document.documentElement.lang === "zh-CN" ? I18N.zh["srv.copy"] : btn.dataset.enAria!;
  btn.addEventListener("click", () => {
    const done = () => {
      btn.classList.add("copied");
      btn.title = label();
      clearTimeout((btn as any)._t);
      (btn as any)._t = setTimeout(() => {
        btn.classList.remove("copied");
        btn.title = restore();
      }, 1500);
    };
    if (navigator.clipboard && window.isSecureContext) {
      navigator.clipboard.writeText(text).then(done, () => {
        if (fallbackCopy(text)) done();
      });
    } else if (fallbackCopy(text)) {
      done();
    }
  });
});
function fallbackCopy(text: string): boolean {
  const ta = document.createElement("textarea");
  ta.value = text;
  ta.style.position = "fixed";
  ta.style.opacity = "0";
  document.body.appendChild(ta);
  ta.select();
  const ok = document.execCommand("copy");
  ta.remove();
  return ok;
}

// 进场动画:IntersectionObserver 一次性加 .in,css 过渡负责动效
const io = new IntersectionObserver(
  (entries) => entries.forEach((e) => e.isIntersecting && (e.target.classList.add("in"), io.unobserve(e.target))),
  { threshold: 0.12 },
);
document.querySelectorAll(".rv").forEach((el) => io.observe(el));

// 下载计数:site/data/hub-stats.json 由每周的 hub-stats 工作流刷新并随站点
// 部署。取不到就保持隐藏——页面不显示一个它无法佐证的数字。
fetch("/data/hub-stats.json", { headers: { accept: "application/json" } })
  .then((r) => (r.ok ? r.json() : Promise.reject(r.status)))
  .then((stats: { totals?: { estimate_minus_ci_floor?: number } }) => {
    const n = stats?.totals?.estimate_minus_ci_floor;
    if (!Number.isFinite(n as number)) return;
    dlCountValue = n as number;
    const el = document.getElementById("dlCount")!;
    const lang = document.documentElement.lang === "zh-CN" ? "zh" : "en";
    const tpl = lang === "zh" ? I18N.zh["dl.count"] : el.dataset.en!;
    el.innerHTML = tpl.replace("{n}", (n as number).toLocaleString("en-US"));
    el.hidden = false;
  })
  .catch(() => {});

// ── 策略示意:光路与流量 ──────────────────────────────────────────────
// 坐标不写死——三个列的布局交给网格,坐标在这里量出来。每条路径两层动画:
// 行进层(亮段沿路径爬行)与可见窗口(hub→Provider 这一腿何时在服务,见
// LEG2_SCHEDULE)。两层都由这里生成成 px/百分比写死的 keyframes:含 var()
// 的 keyframes 在 Chrome 对 stroke-dashoffset 不插值(只能离散跳),而窗口
// 百分比若留在 CSS 里就会和路径长度脱钩——布局一改,流量出现的时刻和它爬到
// Provider 的时刻就对不上(2026-09-30 前正是如此:hub→Provider 要等到周期
// 过了六成才亮,前段四张图全是死路)。

const FLOW_SPEED = 480; // px/s,两腿共用:请求过 hub 时视觉速度不变
const DASH = 20;        // 亮段长度,也是 dasharray 的 dash
const FADE = 1;         // % 的淡入淡出:切换不硬切,窗口仍不重叠

// 谁在服务:hub→Provider 这一腿的排期,单位是 8s 周期的百分比。任何时刻只有
// 一家在服务;切换点落在"当前这家退场"的那一拍上——failover 变红、timewindow
// 扫过窗口、quota 触顶。这些数字与 styles.css 里 provider 名字的颜色、配额环
// 是同一个故事钟,改要一起改。
const LEG2_SCHEDULE: Record<string, Record<string, [number, number]>> = {
  failover: { leg2a: [0, 40], leg2b: [41, 100] },
  timewindow: { leg2a: [0, 33], leg2b: [34, 100] },
  quota: { leg2a: [0, 42], leg2b: [43, 100] },
  // 轮询:三条线轮流接流量,各占约一角;轮到谁谁的线亮(rr-w*)
  roundrobin: { leg2a: [0, 32], leg2b: [34, 65], leg2c: [67, 100] },
};
const LEG2_INDEX: Record<string, number> = { a: 0, b: 1, c: 2 };

// 每张图生成的 keyframes 文本,按图下标缓存。resize 只在几何真的变了时才重写:
// 重设 animationName 会把 8s 的故事钟拨回 0,视口高度一点抖动就让整页动画跳帧
// (旧实现每次 resize 都重建 style 元素,正是这个毛病)。
const flowCss = new Map<number, string>();

function drawStrategyWires(): boolean {
  let changed = false;
  document.querySelectorAll<HTMLElement>(".grid-strat .diag").forEach((fig, i) => {
    const svg = fig.querySelector("svg.wires");
    if (!svg) return;
    const r = fig.getBoundingClientRect();
    svg.setAttribute("viewBox", `0 0 ${Math.round(r.width)} ${Math.round(r.height)}`);
    const c = (el: Element) => {
      const b = el.getBoundingClientRect();
      return [b.left - r.left + b.width / 2, b.top - r.top + b.height / 2];
    };
    const [ax, ay] = c(fig.querySelector(".agent-chip")!);
    const [hx, hy] = c(fig.querySelector(".hub")!);
    const rows = [...fig.querySelectorAll(".provs .pn")].map(c);
    const path = (x1: number, y1: number, x2: number, y2: number) =>
      `M ${x1} ${y1} C ${(x1 + x2) / 2} ${y1}, ${(x1 + x2) / 2} ${y2}, ${x2} ${y2}`;

    // 几何指纹:列宽或任一 logo 的位置动了才重画
    const geom = [ax, ay, hx, hy, ...rows.flat()].map((n) => Math.round(n)).join(",");
    if (fig.dataset.flowGeom === geom) return;
    fig.dataset.flowGeom = geom;
    changed = true;

    // 三条暗光路:agent→hub、hub→每个 provider。
    svg.querySelectorAll<SVGPathElement>(".wire").forEach((w, k) => {
      if (!rows[k]) { w.style.display = "none"; return; }
      w.setAttribute("d", k === 0
        ? path(ax, ay, hx, hy)
        : path(hx, hy, rows[k - 1][0], rows[k - 1][1]));
      w.style.setProperty("--len", String(Math.ceil(w.getTotalLength()) + 2));
    });

    // 流量:leg1 沿 agent→hub 常走;leg2x 沿 hub→第 x 条 provider 线,可见窗口
    // 由排期表决定——同一时刻只有一家在服务。
    const schedule = LEG2_SCHEDULE[fig.dataset.strategy ?? ""] ?? {};
    let css = "";
    svg.querySelectorAll<SVGPathElement>(".flow").forEach((f) => {
      const kind = f.dataset.flow!;
      const target = kind === "leg1"
        ? [ax, ay] as [number, number]
        : rows[LEG2_INDEX[kind.slice(-1)] ?? 0];
      if (!target) { f.style.display = "none"; return; }
      f.setAttribute("d", kind === "leg1"
        ? path(ax, ay, hx, hy)
        : path(hx, hy, target[0], target[1]));

      const len = Math.ceil(f.getTotalLength());
      f.style.strokeDasharray = `${DASH} ${len}`;
      // 行进层:一个周期恰好走完 dash + 路径长 = dasharray 的图案周期,接缝处
      // 严丝合缝(多走一截会在每个循环末端抖一下)。速度两腿一致,长度决定耗时。
      css += `@keyframes flow-run-${i}-${kind} { from { stroke-dashoffset: ${DASH}px; } to { stroke-dashoffset: ${-len}px; } }\n`;
      const parts = [`flow-run-${i}-${kind} ${(len + DASH) / FLOW_SPEED}s linear infinite`];

      // 可见窗口(只有第二腿有):窗口之外整条线是暗的,读到的就是"现在是谁在服务"。
      const win = schedule[kind];
      if (win) {
        const [from, to] = win;
        const head = from > 0
          ? `0%, ${from}% { opacity: 0; } ${Math.min(from + FADE, to)}% { opacity: 1; } `
          : `0% { opacity: 1; } `;
        const tail = to < 100
          ? `${Math.max(to - FADE, from)}% { opacity: 1; } ${to}%, 100% { opacity: 0; }`
          : `100% { opacity: 1; }`;
        css += `@keyframes flow-vis-${i}-${kind} { ${head}${tail} }\n`;
        parts.push(`flow-vis-${i}-${kind} 8s linear infinite`);
      }
      f.style.animation = parts.join(", ");
    });
    flowCss.set(i, css);
  });

  if (!changed) return false;
  const css = [...flowCss.entries()].sort((a, b) => a[0] - b[0]).map(([, text]) => text).join("");
  document.getElementById("flow-kf")?.remove();
  if (css) {
    const st = document.createElement("style");
    st.id = "flow-kf";
    st.textContent = css;
    document.head.appendChild(st);
  }
  return true;
}
window.addEventListener("resize", drawStrategyWires);
drawStrategyWires();

// 实时演示:滚到可见时再把 app 装进 iframe。放在这里而不是首屏,是因为它是
// 一个完整的 React 应用(几百 KB),而首屏只需要那张静态图——没有 JS 的访客
// 就一直看那张图。
const liveHost = document.querySelector<HTMLElement>(".shot-live-host");
if (liveHost) {
  const fig = document.querySelector(".shot-live");
  const loadDemo = () => {
    const frame = document.createElement("iframe");
    frame.title = "Kiwano, running in your browser";
    frame.loading = "lazy";
    // 不加 sandbox:它会把 iframe 变成独立源,而 ES 模块与样式表在同源策略下
    // 加载,独立源里会被 CORS 直接挡掉(实测:整个演示白屏)。内容是我们自己
    // 的静态页面,与父页同源,本来就没有需要隔离的第三方代码。
    frame.src = liveHost.dataset.demoSrc!;
    frame.addEventListener("load", () => document.querySelector(".shot-live")?.classList.add("is-live"));
    liveHost.appendChild(frame);

    // 滚出视口就告诉演示暂停:它在跑一个真实的 React 应用,没人看的时候不该
    // 每秒重渲染一次。用 '*' 是因为同源判断由接收方做(receiver 会校验 origin)。
    const tell = (visible: boolean) => frame.contentWindow?.postMessage({ type: "kiwano-demo", visible }, window.location.origin);
    const liveIo = new IntersectionObserver(
      (entries) => entries.forEach((e) => tell(e.isIntersecting)),
      { threshold: 0 },
    );
    liveIo.observe(fig!);
  };
  const demoIo = new IntersectionObserver(
    (entries) => entries.forEach((e) => { if (e.isIntersecting) { demoIo.disconnect(); loadDemo(); } }),
    { rootMargin: "400px" },
  );
  demoIo.observe(liveHost);
}