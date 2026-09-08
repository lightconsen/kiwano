// Provider brand logo with letter-avatar fallback.
// Icons come from the registry ported from cc-switch (see ./index.ts) —
// inline SVGs size to 1em (set fontSize = box) and use brand fills or
// currentColor; URL icons are bundled raster/SVG assets.
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

  if (icon && hasIcon(icon)) {
    if (isUrlIcon(icon)) {
      return (
        <img
          src={getIconUrl(icon)}
          alt={name}
          className={`shrink-0 rounded-[5px] object-contain ${className}`}
          style={box}
        />
      );
    }
    const meta = getIconMetadata(icon);
    const tint = color || (meta && meta.defaultColor !== "currentColor" ? meta.defaultColor : "") || "var(--ink)";
    return (
      <span
        aria-label={name}
        className={`inline-flex shrink-0 items-center justify-center ${className}`}
        style={{ ...box, fontSize: size, color: tint }}
        dangerouslySetInnerHTML={{ __html: getIcon(icon) }}
      />
    );
  }

  return (
    <div className={`${FALLBACK_CLASS} ${className}`}>
      {name.slice(0, 1).toUpperCase()}
    </div>
  );
}
