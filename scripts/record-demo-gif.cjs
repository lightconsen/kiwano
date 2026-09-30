// Record the site's live demo (site/demo/) as a GIF: a short pass through the
// app — Apps, Models, Dashboard, Settings, back to Apps, dismissing the
// credential banner — with a fake cursor, because a headless recording has no
// pointer of its own.
//
// This drives the *demo build*, not the dev app: the same screens a visitor
// reaches from the landing page, over sample data, with the simulator running
// so the counters move while the camera is on them.
//
// Needs: a local server over site/ (the demo is module-scripted — file:// will
// not load it), playwright, ffmpeg. Usage:
//
//   python3 -m http.server 8899 --directory site &
//   node scripts/record-demo-gif.cjs           # → site/assets/demo.gif + .mp4
//
// `.cjs`, not `.js`: the root package.json is `"type": "module"` for the
// scripts/*.mjs tooling, and this file is CommonJS — under the .js extension
// node reads it as ESM and `require` is not defined.
//
// It writes the two artifacts the repo keeps, from one recording: the GIF
// (720px, 8fps — small enough to embed) and an MP4 of the same pass, a tenth
// of the size, for anywhere that takes video. Env: DEMO_URL (default
// http://127.0.0.1:8899/demo/), OUT (default site/assets/demo.gif).

const path = require("path");
const fs = require("fs");
const os = require("os");
const { execFileSync } = require("child_process");
const { chromium } = require(process.env.PLAYWRIGHT ||
  "/Users/lando/.hermes/hermes-agent/node_modules/playwright");

const BASE = process.env.DEMO_URL || "http://127.0.0.1:8899/demo/";
const OUT = path.resolve(process.env.OUT || "site/assets/demo.gif");
const WIDTH = Number(process.env.WIDTH || 720);
const VIEW = { width: 900, height: 600 }; // the app's own window
const FPS = 8;

/** The fake cursor: a dot that eases to wherever the next click will land. */
const CURSOR_CSS = `
#kw-cursor{position:fixed;z-index:99999;left:0;top:0;width:16px;height:16px;border-radius:50%;
  background:rgba(255,255,255,.95);border:2px solid rgba(0,0,0,.55);box-shadow:0 2px 10px rgba(0,0,0,.6);
  pointer-events:none;opacity:0;transition:transform .42s cubic-bezier(.33,.1,.2,1),opacity .25s ease;
  transform:translate(-50%,-50%)}
#kw-cursor.on{opacity:1}
#kw-cursor.down{width:12px;height:12px;background:rgba(190,255,150,.95)}
`;

async function installCursor(page) {
  await page.addStyleTag({ content: CURSOR_CSS });
  await page.evaluate(() => {
    const el = document.createElement("div");
    el.id = "kw-cursor";
    document.body.appendChild(el);
  });
}

/** Move the pointer onto `selector`, the way a hand would: over half a second,
 *  then a beat before the click actually lands. */
async function pointAt(page, selector) {
  const box = await page.locator(selector).first().boundingBox();
  if (!box) throw new Error(`nothing to point at: ${selector}`);
  const x = box.x + box.width / 2;
  const y = box.y + box.height / 2;
  await page.evaluate(
    ([x, y]) => {
      const el = document.getElementById("kw-cursor");
      el.classList.add("on");
      el.style.transform = `translate(${x}px, ${y}px) translate(-50%, -50%)`;
    },
    [x, y],
  );
  return { x, y };
}

async function clickThrough(page, selector, settle = 1800) {
  const { x, y } = await pointAt(page, selector);
  await page.waitForTimeout(450);
  await page.evaluate(() => document.getElementById("kw-cursor").classList.add("down"));
  await page.mouse.click(x, y);
  await page.waitForTimeout(140);
  await page.evaluate(() => document.getElementById("kw-cursor").classList.remove("down"));
  await page.waitForTimeout(settle);
}


/** Scroll the screen area to its bottom, smoothly, the way a reader would. */
async function scrollToBottom(page, settle = 1100) {
  await page.evaluate(() => {
    const main = document.querySelector("main");
    if (main) main.scrollTo({ top: main.scrollHeight, behavior: "smooth" });
  });
  await page.waitForTimeout(settle);
}

async function main() {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "kiwano-gif-"));
  const browser = await chromium.launch();
  const context = await browser.newContext({
    viewport: VIEW,
    deviceScaleFactor: 1,
    recordVideo: { dir: tmp, size: VIEW },
  });
  const page = await context.newPage();
  page.on("pageerror", (e) => console.error("page error:", e.message));
  // The demo's tick is five seconds — right for a page nobody is watching, and
  // too slow for a clip that has to show the counters moving. The recorder asks
  // for a faster one; the site never does (see src/demo/simulate.ts).
  await page.addInitScript(() => {
    window.__KIWANO_DEMO_TICK_MS__ = 1200;
  });

  await page.goto(BASE, { waitUntil: "networkidle" });
  await page.waitForSelector("nav button", { timeout: 15000 });
  await installCursor(page);

  // The app's own chrome first: the tab strip, then the provider rows — the two
  // things a reader is being shown.
  const tab = (name) =>
    `nav button:text-is("${name}")`;

  // Apps, and a long look at it: the numbers move on their own every few
  // seconds, and the point of this stop is letting a reader watch that happen.
  await pointAt(page, "text=DeepSeek");
  await page.waitForTimeout(3600);
  await pointAt(page, "text=Kimi (Moonshot)");
  await page.waitForTimeout(3600);

  // The other screens: a pass down each, no dwell — what they show is the
  // shape of the thing, and the scroll is what shows it.
  await clickThrough(page, tab("Models"), 900);
  await scrollToBottom(page);

  await clickThrough(page, tab("Dashboard"), 1000);
  await scrollToBottom(page);

  await clickThrough(page, tab("Settings"), 900);
  await scrollToBottom(page);

  await clickThrough(page, tab("Apps"), 900);
  await pointAt(page, "text=DeepSeek");
  await page.waitForTimeout(900);

  // The credential banner is the one thing on screen that is noise in a demo.
  await clickThrough(page, 'button[aria-label="Dismiss"]', 1400);

  await context.close(); // the video is written on close
  await browser.close();

  const webm = fs.readdirSync(tmp).find((f) => f.endsWith(".webm"));
  if (!webm) throw new Error("no video was recorded");
  const video = path.join(tmp, webm);
  console.log("recorded:", video, fs.statSync(video).size, "bytes");

  // GIF, the way ffmpeg wants it: build a palette from the video first, then
  // render through it — a single pass over a dark UI with a green accent
  // dithers into mud.
  fs.mkdirSync(path.dirname(OUT), { recursive: true });
  const palette = path.join(tmp, "palette.png");
  const fps = `fps=${FPS},scale=${WIDTH}:-1:flags=lanczos`;
  execFileSync("ffmpeg", ["-y", "-i", video, "-vf", `${fps},palettegen=stats_mode=diff`, palette]);
  execFileSync("ffmpeg", [
    "-y",
    "-i",
    video,
    "-i",
    palette,
    "-lavfi",
    `${fps}[x];[x][1:v]paletteuse=dither=none:diff_mode=rectangle`,
    "-loop",
    "0",
    OUT,
  ]);
  console.log("wrote:", OUT, (fs.statSync(OUT).size / 1024 / 1024).toFixed(1), "MB");

  // The same pass as video: every platform takes MP4 and re-encodes a GIF into
  // something worse, so this is what goes wherever a clip can be uploaded.
  // Height is even because H.264 requires it, and 600/900 scales to exactly
  // 480 at this width.
  const mp4 = OUT.replace(/\.gif$/, ".mp4");
  execFileSync("ffmpeg", [
    "-y",
    "-i",
    video,
    "-vf",
    `scale=${WIDTH}:${Math.round((WIDTH * VIEW.height) / VIEW.width / 2) * 2}`,
    "-movflags",
    "+faststart",
    "-pix_fmt",
    "yuv420p",
    "-crf",
    "24",
    mp4,
  ]);
  console.log("wrote:", mp4, (fs.statSync(mp4).size / 1024 / 1024).toFixed(1), "MB");
  fs.rmSync(tmp, { recursive: true, force: true });
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
