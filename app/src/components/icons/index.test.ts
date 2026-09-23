// Registry contract (KIW-SEC-001): every provider icon is a *local* asset.
//
// `ProviderLogo` renders only what the registries in `./index` return — inline
// SVG strings through `dangerouslySetInnerHTML`, bundled images through a
// URL. Nothing remote ever reaches either path today. These tests pin that
// contract, so a future "add a remote logo" change breaks here, in the test
// suite, before it can reach the WebView.
import { describe, expect, it } from "vitest";
import { darkIconUrls, icons, iconUrls } from "./index";

import { AGENT_ICON } from "../bits";
import { NO_CLI_AGENTS, SEGMENT_ICON } from "../../screens/Providers/agents";
import { AGENTS, type AgentId } from "../../api/types";

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

  // Two agent→icon tables must agree with the agent registry, and this is the
  // only place that can hold them to it: the strip's table and the
  // declare-menu's table were maintained by hand, and adding an agent meant
  // remembering both — the one memory failed at twice (mimo's icon shipped in
  // one table and not the other, which is this test's reason to exist). The
  // GUI-only agents have no CLI binary and, by the same mark, no ported logo.
  it("names an icon for every CLI agent, in both agent→icon tables", () => {
    for (const agent of AGENTS) {
      // AGENTS is the built-in registry, so every id is a closed AgentId at
      // runtime — the same assertion Providers.tsx's declare menu makes.
      if (NO_CLI_AGENTS.includes(agent.id as AgentId)) continue;
      const segIcon = SEGMENT_ICON[agent.id];
      const menuIcon = AGENT_ICON[agent.id];
      expect(segIcon, `${agent.id} has a strip icon`).toBeTruthy();
      expect(menuIcon, `${agent.id} has a declare-menu icon`).toBeTruthy();
      for (const icon of [segIcon, menuIcon]) {
        const key = icon!.toLowerCase();
        expect(key in icons || key in iconUrls, `${agent.id}: "${icon}" is in the registry`).toBe(
          true,
        );
      }
    }
  });
});
