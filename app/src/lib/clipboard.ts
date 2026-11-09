// Putting a string on the system clipboard.
//
// Inside the app the write is native (`src-tauri/src/clipboard.rs`), and the
// reason is measured rather than stylistic: WKWebView has no Clipboard API, so
// the web route is `execCommand("copy")` over a selected `<textarea>` — and
// WebKit serializes that selection on the main thread. A request log's Copy
// carries its bodies, and a coding agent's request is routinely several
// megabytes (the gateway stores bodies in full by default). Measured in a real
// WKWebView: 2 MB froze the window for 0.5 s, 7 MB for 6.8 s, 14 MB for 37 s.
// The OS pasteboard takes the same string in one call.
import { invoke } from "@tauri-apps/api/core";

/** The test `api/client.ts` makes to pick its transport: outside Tauri there is
    no `__TAURI_INTERNALS__`, and `invoke` would reject. */
const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

/** Past this, the web path is refused rather than attempted. `execCommand`'s
    cost grows faster than the string it is given, and a copy that freezes the
    page is worse than one that reports it failed. Same reasoning — and the same
    number — as `BODY_RENDER_CAP` in `screens/RequestLogs.tsx`, which bounds the
    rendering side of the same webview. */
const WEB_COPY_CAP = 256 * 1024;

/** Copy `text`; resolves whether the clipboard took it. Never rejects — every
    caller has a "Copy failed" answer and nothing else it could do. */
export async function copyText(text: string): Promise<boolean> {
  if (inTauri) {
    try {
      await invoke("copy_text", { text });
      return true;
    } catch {
      return false;
    }
  }
  const clip = navigator.clipboard;
  if (clip) {
    try {
      await clip.writeText(text);
      return true;
    } catch {
      // No permission, or no focus: the legacy path below is still worth a try.
    }
  }
  if (text.length > WEB_COPY_CAP) return false;
  return legacyCopy(text);
}

/** The pre-Clipboard-API route: a selected textarea and the document's own copy
    command. Synchronous, because that is the only way a webview honours it —
    and the reason the app writes through the OS instead. */
function legacyCopy(text: string): boolean {
  try {
    const ta = document.createElement("textarea");
    ta.value = text;
    ta.style.position = "fixed";
    ta.style.opacity = "0";
    document.body.appendChild(ta);
    ta.select();
    const ok = document.execCommand("copy");
    ta.remove();
    return ok;
  } catch {
    return false;
  }
}
