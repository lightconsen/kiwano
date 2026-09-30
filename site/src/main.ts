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

// 策略示意图的请求点:路标坐标不写死——三个列的布局交给网格,坐标在这里
// 量出来写到 .rq 的 CSS 变量上。resize 不重测:示意图宽度固定在两列网格里,
// 而 keyframes 引用的是变量,量一次就够。
function drawStrategyWires() {
  document.querySelectorAll<HTMLElement>(".grid-strat .diag").forEach((fig) => {
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
    // 三条暗光路:agent→hub、hub→每个 provider。
    svg.querySelectorAll<SVGPathElement>(".wire").forEach((w, i) => {
      if (!rows[i]) { w.style.display = "none"; return; }
      w.setAttribute("d", i === 0
        ? path(ax, ay, hx, hy)
        : path(hx, hy, rows[i - 1][0], rows[i - 1][1]));
      const len = Math.ceil(w.getTotalLength()) + 2;
      w.style.setProperty("--len", String(len));
    });
    // 流量:leg1 沿 agent→hub;leg2 沿 hub→命中的 provider(每个可能的命中者
    // 一条 path,配对各自的 keyframes)。
    svg.querySelectorAll<SVGPathElement>(".flow").forEach((f) => {
      const kind = f.dataset.flow!;
      if (kind === "leg1") {
        f.setAttribute("d", path(ax, ay, hx, hy));
      } else {
        // leg2a / leg2b:第几条 provider 线。roundrobin 只有一条 leg2,交由
        // rr-f 的多段 keyframes 在三条线间换 —— 那需要每帧换 d,退而求其次:
        // rr 的 leg2 用一条"hub→中线"的合成路径近似,命中的线靠 wire 亮起表达。
        const idx = kind.endsWith("b") ? 1 : 0;
        const target = rows[idx] || rows[0];
        if (!target) { f.style.display = "none"; return; }
        f.setAttribute("d", path(hx, hy, target[0], target[1]));
      }
      const len = Math.ceil(f.getTotalLength()) + 2;
      f.style.setProperty("--len", String(len));
      f.style.strokeDasharray = `20 ${len}`;
    });
  });
  writeFlowKeyframes();
}

// 每条 leg1 的"往"程动画写成专属 keyframes——距离写死为该路径的像素长。
// (含 var() 的 keyframes 在 Chrome 对 stroke-dashoffset 不插值,只能离散跳,
// 所以不能共用一条带变量的动画。leg2 的 keyframes 是"到达点"语义,跳变
// 无妨,继续共用。)
function writeFlowKeyframes() {
  let css = "";
  document.querySelectorAll<HTMLElement>(".grid-strat .diag").forEach((fig, i) => {
    const leg1 = fig.querySelector('[data-flow="leg1"]');
    if (leg1) {
      const len = Math.ceil((leg1 as SVGPathElement).getTotalLength()) + 40;
      css += `@keyframes flow-leg1-${i} { from { stroke-dashoffset: 0; } to { stroke-dashoffset: -${len}px; } }\n`;
      leg1.style.animationName = `flow-leg1-${i}`;
      leg1.style.animationDuration = "0.4s";
    }
    // leg2 的行进层:每条可能的命中路径一条,0.4s 走完自身——与 leg1 同速。
    fig.querySelectorAll<SVGPathElement>(".flow[data-flow^='leg2']").forEach((f, j) => {
      const len = Math.ceil(f.getTotalLength()) + 20;
      css += `@keyframes leg2-run-${i}-${j} { from { stroke-dashoffset: 0; } to { stroke-dashoffset: -${len}px; } }\n`;
      f.style.animationName = `leg2-run-${i}-${j}, ${f.classList.contains("fa-f2a") ? "fa-f2a-vis" : f.classList.contains("fa-f2b") ? "fa-f2b-vis" : f.classList.contains("tw-f2a") ? "tw-f2a-vis" : f.classList.contains("tw-f2b") ? "tw-f2b-vis" : "fa-f2a-vis"}`;
      f.style.animationDuration = "0.4s, 8s";
    });
  });
  document.getElementById("flow-kf")?.remove();
  if (css) {
    const st = document.createElement("style");
    st.id = "flow-kf";
    st.textContent = css;
    document.head.appendChild(st);
  }
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