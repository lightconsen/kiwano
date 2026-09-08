// Provider brand logo with letter-avatar fallback.
// Icons come from the registry ported from cc-switch (see ./index.ts) —
// inline SVGs size to 1em (set fontSize = box) and use brand fills or
// currentColor; URL icons are bundled raster/SVG assets.
// Near-black marks have a dark-theme variant asset (darkIconUrls): under
// html.dark the light version renders, otherwise the original.
import { useEffect, useState } from "react";
import { getIcon, getIconUrl, getDarkIconUrl, hasDarkVariant, hasIcon, isUrlIcon } from "./index";
import { getIconMetadata } from "./metadata";

const FALLBACK_CLASS = "logo-c w-6 h-6 text-[11px]";

function useIsDarkTheme(): boolean {
  const [dark, setDark] = useState(() =>
    typeof document !== "undefined" && document.documentElement.classList.contains("dark"),
  );
  useEffect(() => {
    const el = document.documentElement;
    const ob = new MutationObserver(() => setDark(el.classList.contains("dark")));
    ob.observe(el, { attributes: true, attributeFilter: ["class"] });
    return () => ob.disconnect();
  }, []);
  return dark;
}

export function ProviderLogo({
  icon,
  name,
  color,
  char,
  size = 24,
  className = "",
}: {
  icon?: string | null;
  name: string;
  color?: string;
  /** Fallback letter override (claude-desktop renders "D", not "C") */
  char?: string;
  size?: number;
  className?: string;
}) {
  const dark = useIsDarkTheme();
  const box: React.CSSProperties = { width: size, height: size };
  const useDark = dark && !!icon && hasDarkVariant(icon);

  if (icon && hasIcon(icon)) {
    if (isUrlIcon(icon) || useDark) {
      const src = useDark ? getDarkIconUrl(icon) : getIconUrl(icon);
      return (
        <img
          src={src}
          alt={name}
          className={`shrink-0 rounded-[5px] object-contain ${className}`}
          style={box}
        />
      );
    }
    const meta = getIconMetadata(icon);
    const tint = color || (meta && meta.defaultColor !== "currentColor" ? meta.defaultColor : "") || "var(--ink)";
    // The ported SVGs carry their own <title> (e.g. "Claude"), which the
    // browser shows as a native tooltip overriding ours — strip it; this
    // component already exposes the name via aria-label.
    const svg = getIcon(icon).replace(/<title>[\s\S]*?<\/title>/g, "");
    return (
      <span
        aria-label={name}
        className={`inline-flex shrink-0 items-center justify-center ${className}`}
        style={{ ...box, fontSize: size, color: tint }}
        dangerouslySetInnerHTML={{ __html: svg }}
      />
    );
  }

  return (
    <div
      className={`${FALLBACK_CLASS} ${className}`}
      style={{
        width: size,
        height: size,
        fontSize: Math.max(10, Math.round(size * 0.45)),
        background: color || "var(--surface2)",
        border: "1px solid var(--line)",
      }}
    >
      {(char ?? name.slice(0, 1)).toUpperCase()}
    </div>
  );
}
