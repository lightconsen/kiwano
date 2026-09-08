// Provider brand logo with letter-avatar fallback.
// Icons come from the registry ported from cc-switch (see ./index.ts) —
// inline SVGs size to 1em (set fontSize = box) and use brand fills or
// currentColor; URL icons are bundled raster/SVG assets.
// Near-black brand marks (metadata.darkMark) render on a light tile so
// they stay visible on dark surfaces.
import { getIcon, getIconUrl, hasIcon, isUrlIcon } from "./index";
import { getIconMetadata } from "./metadata";

const FALLBACK_CLASS = "logo-c w-6 h-6 text-[11px]";

export function ProviderLogo({
  icon,
  name,
  color,
  size = 24,
  className = "",
}: {
  icon?: string | null;
  name: string;
  color?: string;
  size?: number;
  className?: string;
}) {
  const box: React.CSSProperties = { width: size, height: size };
  const meta = icon ? getIconMetadata(icon) : undefined;
  const tile: React.CSSProperties | undefined = meta?.darkMark
    ? { background: "var(--logo-tile)", borderRadius: 5, padding: Math.max(2, size * 0.08) }
    : undefined;

  if (icon && hasIcon(icon)) {
    if (isUrlIcon(icon)) {
      return (
        <span className={`inline-flex shrink-0 ${className}`} style={{ ...box, ...tile }}>
          <img src={getIconUrl(icon)} alt={name} className="object-contain" style={{ width: "100%", height: "100%" }} />
        </span>
      );
    }
    const tint = color || (meta && meta.defaultColor !== "currentColor" ? meta.defaultColor : "") || "var(--ink)";
    return (
      <span
        aria-label={name}
        className={`inline-flex shrink-0 items-center justify-center ${className}`}
        style={{ ...box, ...tile, fontSize: size, color: tint }}
        dangerouslySetInnerHTML={{ __html: getIcon(icon) }}
      />
    );
  }

  return (
    <div
      className={`${FALLBACK_CLASS} ${className}`}
      style={{
        background: color || "var(--surface2)",
        border: "1px solid var(--line)",
      }}
    >
      {name.slice(0, 1).toUpperCase()}
    </div>
  );
}
