// Registry contract (KIW-SEC-001): every provider icon is a *local* asset.
//
// `ProviderLogo` renders only what the registries in `./index` return — inline
// SVG strings through `dangerouslySetInnerHTML`, bundled images through a
// URL. Nothing remote ever reaches either path today. These tests pin that
// contract, so a future "add a remote logo" change breaks here, in the test
// suite, before it can reach the WebView.
import { describe, expect, it } from "vitest";
import { darkIconUrls, icons, iconUrls } from "./index";

describe("provider icon registry", () => {
  it("sources every icon from a local asset, never a remote URL", () => {
    const urls = [
      ...Object.entries(iconUrls),
      ...Object.entries(darkIconUrls),
    ];
    for (const [key, url] of urls) {
      expect(
        url.startsWith("http://") || url.startsWith("https://"),
        `${key} resolves to a remote URL: ${url}`,
      ).toBe(false);
    }
  });

  it("keeps inline SVGs free of scripts and event handlers", () => {
    for (const [key, svg] of Object.entries(icons)) {
      expect(
        svg.trimStart().startsWith("<svg"),
        `${key} is not a bare <svg> element`,
      ).toBe(true);
      expect(svg, `${key} embeds a <script>`).not.toMatch(/<script/i);
      expect(
        svg,
        `${key} carries an inline event handler (on*)`,
      ).not.toMatch(/\son\w+\s*=/i);
      expect(svg, `${key} references javascript:`).not.toMatch(/javascript\s*:/i);
    }
  });
});
