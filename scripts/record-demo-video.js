// Records a ~90s Kiwano demo video from the real dev build + built-in sample data.
// Subtitles and a fake cursor are injected into the page, so no post-editing is needed.
const { chromium } = require("playwright");
const fs = require("fs");
const path = require("path");

const BASE = "http://localhost:1420/";
const OUT_DIR = "/tmp/kiwano-video";
const W = 1600;
const H = 1000;

// ---- injected overlay: splash / caption / cursor -----------------------------
function installOverlay() {
  const css = `
#kw-splash{position:fixed;inset:0;z-index:99999;background:#07090d;display:flex;flex-direction:column;
  align-items:center;justify-content:center;gap:18px;opacity:0;transition:opacity .55s ease;pointer-events:none;}
#kw-splash .k{font:800 104px/1 -apple-system,"SF Pro Display",Inter,"Helvetica Neue",Arial,sans-serif;
  letter-spacing:.14em;color:#fff;}
#kw-splash .s{font:600 30px/1.4 -apple-system,"SF Pro Display",Inter,"Helvetica Neue",Arial,sans-serif;
  letter-spacing:.05em;color:#9aa4b2;text-align:center;}
#kw-splash .f{font:600 22px/1.4 -apple-system,Inter,Arial,sans-serif;color:#6b7684;margin-top:12px;letter-spacing:.08em;}
#kw-cap-wrap{position:fixed;left:0;right:0;bottom:0;z-index:99998;display:flex;justify-content:center;
  padding:0 0 44px;pointer-events:none;}
#kw-cap{font:800 46px/1.3 -apple-system,"SF Pro Display",Inter,"Helvetica Neue",Arial,sans-serif;
  letter-spacing:.02em;color:#fff;text-align:center;max-width:1240px;padding:16px 34px;border-radius:16px;
  background:rgba(7,9,13,.82);text-shadow:0 2px 12px rgba(0,0,0,.95);opacity:0;transition:opacity .3s ease;}
#kw-cursor{position:fixed;z-index:99997;left:0;top:0;width:17px;height:17px;border-radius:50%;
  background:rgba(255,255,255,.92);border:2px solid rgba(0,0,0,.5);box-shadow:0 2px 8px rgba(0,0,0,.55);
  pointer-events:none;transform:translate(-50%,-50%);opacity:0;transition:opacity .2s ease;}
#kw-cursor.down{transform:translate(-50%,-50%) scale(.7);}
`;
  const mount = () => {
    const style = document.createElement("style");
    style.textContent = css;
    document.head.appendChild(style);

    const splash = document.createElement("div");
    splash.id = "kw-splash";
    splash.innerHTML = '<div class="k">KIWANO</div><div class="s"></div><div class="f"></div>';
    document.body.appendChild(splash);

    const wrap = document.createElement("div");
    wrap.id = "kw-cap-wrap";
    const cap = document.createElement("div");
    cap.id = "kw-cap";
    wrap.appendChild(cap);
    document.body.appendChild(wrap);

    const cur = document.createElement("div");
    cur.id = "kw-cursor";
    document.body.appendChild(cur);

    document.addEventListener("mousemove", (ev) => {
      cur.style.left = ev.clientX + "px";
      cur.style.top = ev.clientY + "px";
      cur.style.opacity = "1";
    });
    document.addEventListener("mousedown", () => cur.classList.add("down"));
    document.addEventListener("mouseup", () => cur.classList.remove("down"));
  };

  window.__kwCap = (t) => {
    const e = document.getElementById("kw-cap");
    if (!e) return;
    e.style.opacity = "0";
    setTimeout(() => {
      e.textContent = t;
      e.style.opacity = "1";
    }, 260);
  };
  window.__kwSplash = (on, sub, foot) => {
    const e = document.getElementById("kw-splash");
    if (!e) return;
    if (on) {
      e.querySelector(".s").textContent = sub || "";
      e.querySelector(".f").textContent = foot || "";
      e.style.opacity = "1";
    } else {
      e.style.opacity = "0";
    }
  };

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", mount);
  } else {
    mount();
  }
}

(async () => {
  fs.rmSync(OUT_DIR, { recursive: true, force: true });
  fs.mkdirSync(OUT_DIR, { recursive: true });

  const browser = await chromium.launch();
  const context = await browser.newContext({
    viewport: { width: W, height: H },
    recordVideo: { dir: OUT_DIR, size: { width: W, height: H } },
  });
  await context.addInitScript(installOverlay);
  const page = await context.newPage();

  const t0 = Date.now();
  const at = () => ((Date.now() - t0) / 1000).toFixed(1) + "s";
  // PACE stretches every hold so the finished cut lands at ~90s.
  const PACE = Number(process.env.PACE || 1.6);
  const hold = (ms) => page.waitForTimeout(Math.round(ms * PACE));
  const cap = async (t) => {
    await page.evaluate((x) => window.__kwCap(x), t);
    console.log("CAP", at(), "|", t);
  };
  const splash = async (on, sub, foot) =>
    page.evaluate(([o, s, f]) => window.__kwSplash(o, s, f), [on, sub, foot]);
  const glide = async (loc, steps = 30) => {
    const b = await loc.boundingBox();
    if (!b) return null;
    await page.mouse.move(b.x + b.width / 2, b.y + b.height / 2, { steps });
    return b;
  };
  // Some screens are not <table>-based; fall back to a screen position so a
  // missing locator never aborts a 90-second recording.
  const safeGlide = async (loc, fx, fy, steps = 30) => {
    try {
      const b = await loc.boundingBox();
      if (b) {
        await page.mouse.move(b.x + Math.min(b.width / 2, 200), b.y + b.height / 2, { steps });
        return b;
      }
    } catch {
      /* fall through to the fixed position */
    }
    await page.mouse.move(fx, fy, { steps });
    return null;
  };
  const goHash = async (h) => {
    await page.evaluate((x) => { window.location.hash = x; }, h);
  };

  // ---------- S0 splash ------------------------------------------------------
  await page.goto(BASE + "#providers", { waitUntil: "load" });
  await page.getByRole("button", { name: "Add provider" }).first().waitFor({ timeout: 20000 });
  await page.evaluate(() => document.fonts.ready);
  await hold(600);
  await splash(true, "LOCAL-FIRST AI PROVIDER MANAGER", "");
  await hold(3000);

  // ---------- S1 Apps home ---------------------------------------------------
  await splash(false);
  // Deliberately vague: no port number or host on screen, so the claim can never
  // be contradicted by a future default change.
  await cap("A LOCAL AGENT GATEWAY");
  await hold(1400);
  await safeGlide(page.getByText("DeepSeek").first(), 420, 300, 34);
  await hold(2200);
  await safeGlide(page.getByRole("button", { name: "Add provider" }).first(), 1180, 150, 26);
  await hold(2600);
  console.log("S1 apps done", at());

  // ---------- S2 Models shelf ------------------------------------------------
  await goHash("#shelf");
  await page.locator('input[placeholder="Search providers…"]').first().waitFor({ timeout: 15000 });
  await cap("MODELS SHELF — SYNC ONCE, THEN IT WORKS OFFLINE");
  await hold(1100);
  const inp = page.locator('input[placeholder="Search providers…"]').first();
  await glide(inp, 24);
  await page.mouse.click((await inp.boundingBox()).x + 60, (await inp.boundingBox()).y + 14);
  await inp.type("o", { delay: 260 });
  await hold(1400);
  await cap("PRICING, PROTOCOL AND QUOTA — BEFORE YOU ADD IT");
  await hold(1700);
  console.log("S2 shelf done", at());

  // ---------- S3 Agents bound ------------------------------------------------
  await goHash("#providers/claude");
  await cap("EVERY AGENT, ONE PORT, ONE SET OF KEYS");
  await hold(1300);
  await hold(1500);
  await safeGlide(page.getByText("Claude Code").first(), 430, 300, 26);
  await hold(2200);
  console.log("S3 agents done", at());

  // ---------- S4 Strategy = failover -----------------------------------------
  const strat = page.locator('[role=combobox][aria-label="Claude Code strategy"]').first();
  await strat.waitFor({ timeout: 15000 });
  await cap("FIVE ROUTING STRATEGIES — ONE PER AGENT");
  await glide(strat, 28);
  await hold(900);
  await strat.click();
  await hold(2200);
  await glide(page.getByRole("option", { name: "Failover", exact: true }).first(), 22);
  await hold(1400);
  await cap("FAILOVER: THE REQUEST REPLAYS. THE AGENT NEVER NOTICES.");
  await hold(600);
  await page.getByRole("option", { name: "Failover", exact: true }).first().click();
  await hold(3200);
  console.log("S4 strategy done", at());

  // ---------- S5 Costs -------------------------------------------------------
  await goHash("#dashboard");
  await page.getByText("By provider").first().waitFor({ timeout: 15000 });
  await cap("REAL PER-REQUEST COST — PER AGENT × PROVIDER");
  await hold(2000);
  await safeGlide(page.getByText("Usage trend").first(), 520, 430, 26);
  await hold(1800);
  await safeGlide(page.getByText("By provider").first(), 1150, 430, 24);
  await hold(1600);
  await safeGlide(page.getByText("Data stays in local SQLite").first(), 1450, 68, 24);
  await cap("KEYS NEVER LEAVE YOUR MACHINE");
  await hold(3000);
  console.log("S5 costs done", at());

  // ---------- S6 Request log detail ------------------------------------------
  const logsCard = page
    .locator("div.mt-3.rounded-lg")
    .filter({ has: page.getByRole("heading", { name: "Logs" }) })
    .first();
  const firstRow = logsCard.locator("tbody tr").first();
  await cap("EVERY REQUEST, AUDITABLE");
  await safeGlide(firstRow, W / 2, H - 220, 26);
  await hold(900);
  await firstRow.click();
  await page.locator("[role=dialog]").first().waitFor({ timeout: 10000 });
  await hold(2200);
  await page.mouse.move(W / 2, H / 2 + 60, { steps: 20 });
  await hold(3000);
  console.log("S6 log detail done", at());

  // ---------- S6b Settings -> Features ---------------------------------------
  await page.keyboard.press("Escape");
  await hold(700);
  await goHash("#settings");
  await page.getByText("Optional capabilities built on the request log").first().waitFor({ timeout: 15000 });
  // Note: the dev fixture has some switches on, so never claim "all off" on
  // screen — describe what the features are, not their default state.
  await cap("OPT-IN FEATURES, BUILT ON THE REQUEST LOG");
  await hold(1000);
  // Scroll the Features card into view; Settings is a long page.
  await page.getByText("Optional capabilities built on the request log").first().scrollIntoViewIfNeeded();
  await hold(1500);
  await safeGlide(page.getByText("Cost forecast").first(), 640, 520, 24);
  await hold(1600);
  await safeGlide(page.getByText("Agent self-query (MCP)").first(), 640, 660, 24);
  await hold(1800);
  console.log("S6b features done", at());

  // ---------- S7 outro -------------------------------------------------------
  await page.keyboard.press("Escape");
  await hold(1200);
  await cap("ONE COMMAND TO INSTALL · GPLv3 · MACOS / WINDOWS / LINUX");
  await hold(2600);
  await splash(true, "kiwano.cc", "GPLv3 · SIGNED PROVENANCE ON EVERY RELEASE");
  await hold(5200);
  console.log("S7 outro done", at());

  const video = page.video();
  await page.close();
  await context.close();
  await browser.close();
  const src = await video.path();
  const dst = path.join(OUT_DIR, "kiwano-demo-90s.webm");
  fs.renameSync(src, dst);
  console.log("VIDEO:", dst, fs.statSync(dst).size, "bytes");
  console.log("TOTAL:", at());
})().catch(async (e) => {
  console.error("FAILED:", e.message);
  process.exit(1);
});
