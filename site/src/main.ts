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
  // 演示里跑的是真应用,它有自己的一套语言(设置里的 `language`,默认跟随浏览器)
  // —— 于是英文页面上可能嵌着一个中文演示。页面这一侧是权威:语言一变就告诉
  // 它一声,它按同一门语言重画。(iframe 还没建时这里什么也不会发生,那种情况
  // 由 loadDemo 在 src 上带 ?lang= 处理。)
  document
    .querySelector<HTMLIFrameElement>(".shot-live-host iframe")
    ?.contentWindow?.postMessage({ type: "kiwano-demo", lang: siteLang() }, window.location.origin);
}

/** 页面当前的语言,取值和写进 <html lang> 的那一个。 */
function siteLang(): "en" | "zh-CN" {
  return document.documentElement.lang === "zh-CN" ? "zh-CN" : "en";
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
// (roundrobin 不在这张表里:它的三条流同时跑、各一色,见下面的 stream。)
const LEG2_SCHEDULE: Record<string, Record<string, [number, number]>> = {
  failover: { leg2a: [0, 40], leg2b: [41, 100] },
  timewindow: { leg2a: [0, 33], leg2b: [34, 100] },
  quota: { leg2a: [0, 42], leg2b: [43, 100] },
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
    // 坐标原点取画布而不是卡片:窄屏时卡片是滚动容器,几何要按内容自己的框来
    // 量,否则一滚动连线就错位。宽屏两者同框,取值不变。
    const canvas = fig.querySelector<HTMLElement>(".diag-canvas") ?? fig;
    const r = canvas.getBoundingClientRect();
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
    // 全程一条:agent→hub→provider,中途正好穿过 hub —— roundrobin 的三条流
    // 走这条,看得到"同一批请求被网关摊到三家"。
    const route = (x1: number, y1: number, mx: number, my: number, x2: number, y2: number) =>
      path(x1, y1, mx, my) + path(mx, my, x2, y2).replace(/^M [^C]+/, "");

    // 几何指纹:列宽或任一 logo 的位置动了才重画
    const geom = [ax, ay, hx, hy, ...rows.flat()].map((n) => Math.round(n)).join(",");
    if (fig.dataset.flowGeom === geom) return;
    fig.dataset.flowGeom = geom;
    changed = true;

    // 暗光路:第 0 条是 agent→hub,第 k 条(k≥1)接 rows[k-1] 那家——守卫得看
    // rows[k-1];看 rows[k] 会把最后一条 provider 线永远判成"没人可接"。
    svg.querySelectorAll<SVGPathElement>(".wire").forEach((w, k) => {
      if (k > 0 && !rows[k - 1]) { w.style.display = "none"; return; }
      w.style.display = "";
      w.setAttribute("d", k === 0
        ? path(ax, ay, hx, hy)
        : path(hx, hy, rows[k - 1][0], rows[k - 1][1]));
      w.style.setProperty("--len", String(Math.ceil(w.getTotalLength()) + 2));
    });

    // 流量。三种路径:
    //   leg1   agent→hub,常走
    //   leg2x  hub→第 x 家,可见窗口由排期表决定——同一时刻只有一家在服务
    //   stream agent→hub→第 n 家,整条同时跑(roundrobin:三条各一色,并发)
    const schedule = LEG2_SCHEDULE[fig.dataset.strategy ?? ""] ?? {};
    let css = "";

    const streams = [...svg.querySelectorAll<SVGPathElement>('.flow[data-flow="stream"]')];
    streams.forEach((f, k) => {
      const target = rows[k];
      if (!target) { f.style.display = "none"; return; }
      // 起点沿 agent 这一侧上下错开,三条流在 chip 边上就是三条并行的线,到 hub
      // 汇合后再各走各家;相位各错开三分之一个周期,三条亮段就不会叠成一坨,
      // 一眼看得出是同一批请求被摊到三家。
      const oy = (k - (streams.length - 1) / 2) * 11;
      f.setAttribute("d", route(ax, ay + oy, hx, hy, target[0], target[1]));
      const len = Math.ceil(f.getTotalLength());
      const dur = (len + DASH) / FLOW_SPEED;
      f.style.strokeDasharray = `${DASH} ${len}`;
      css += `@keyframes flow-run-${i}-s${k} { from { stroke-dashoffset: ${DASH}px; } to { stroke-dashoffset: ${-len}px; } }\n`;
      f.style.animation = `flow-run-${i}-s${k} ${dur.toFixed(3)}s linear infinite`;
      f.style.animationDelay = `${(-k * dur / streams.length).toFixed(3)}s`;
    });

    svg.querySelectorAll<SVGPathElement>(".flow").forEach((f) => {
      const kind = f.dataset.flow!;
      if (kind === "stream") return; // 上面已经画过
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
//
// 窄屏不装:演示是桌面应用的 900×600 窗口,塞进手机的 330px 只会渲染出一张
// 读不了的界面(实测:横幅折成四行、provider 行被裁、状态栏压住内容),而那
// 886 KB 的 JS 是要用流量下的。窄屏留静态图,文案换成 .demo-note-narrow,
// 想去玩的人从那里新开一个标签页。
const LIVE_DEMO_MIN_WIDTH = 720;
const liveHost = document.querySelector<HTMLElement>(".shot-live-host");
if (liveHost && window.matchMedia(`(min-width: ${LIVE_DEMO_MIN_WIDTH}px)`).matches) {
  const fig = document.querySelector(".shot-live");
  const loadDemo = () => {
    const frame = document.createElement("iframe");
    frame.title = "Kiwano, running in your browser";
    frame.loading = "lazy";
    // 不加 sandbox:它会把 iframe 变成独立源,而 ES 模块与样式表在同源策略下
    // 加载,独立源里会被 CORS 直接挡掉(实测:整个演示白屏)。内容是我们自己
    // 的静态页面,与父页同源,本来就没有需要隔离的第三方代码。
    // 语言随 src 一起进去:访客多半先在顶部切好语言再滚下来,那时演示还没建,
    // postMessage 那条路是空的。
    frame.src = `${liveHost.dataset.demoSrc}?lang=${siteLang()}`;
    // 换上去的时机不能只听 `load`。iframe 里只要有一个外部脚本永远不返回,
    // 它的 load 事件就永远不触发——Cloudflare 在边缘注入的
    // static.cloudflareinsights.com 信标正是这种脚本,而从中国大陆访问它会
    // 直接超时。结果是:演示其实已经跑起来、已经画出来了,父页却一直把
    // .shot-live-host 留在 opacity: 0,访客看到的还是那张静态图,而 demo 在
    // 后面空转。所以三个信号取先到的:`load`(最准)、iframe 内部真的画出
    // 东西了(同源,可以看 contentDocument)、以及兜底计时器——兜底是为了
    // 万一它哪天不再同源,那时前一个信号会失效。
    const painted = () => {
      try {
        const root = frame.contentDocument?.getElementById("root");
        return !!root && root.childElementCount > 0;
      } catch {
        return false; // 不同源:contentDocument 不可读,交给兜底
      }
    };
    let shown = false;
    const show = () => {
      if (shown) return;
      shown = true;
      window.clearInterval(poll);
      document.querySelector(".shot-live")?.classList.add("is-live");
    };
    const poll = window.setInterval(() => {
      if (painted()) show();
    }, 200);
    frame.addEventListener("load", show);
    window.setTimeout(show, 8000);
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

// 窄屏的导航菜单。开合状态放在按钮的 aria-expanded 上,面板只认 .open——两个
// 读的是同一件事,不会各说各话。点链接就收起来(锚点跳转之后菜单还杵在那儿
// 是移动端最常见的毛病),Esc 也收,转回宽屏时更要收:否则菜单会以展开的样子
// 留在桌面端。
const navBurger = document.getElementById("navBurger");
const navPanel = document.getElementById("navPanel");
if (navBurger && navPanel) {
  const setMenu = (open: boolean) => {
    navPanel.classList.toggle("open", open);
    navBurger.setAttribute("aria-expanded", String(open));
  };
  navBurger.addEventListener("click", () => setMenu(navBurger.getAttribute("aria-expanded") !== "true"));
  navPanel.addEventListener("click", (e) => {
    if ((e.target as HTMLElement).closest("a")) setMenu(false);
  });
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape") setMenu(false);
  });
  window.matchMedia("(min-width: 1081px)").addEventListener("change", (e) => {
    if (e.matches) setMenu(false);
  });
}