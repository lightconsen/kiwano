// Hub catalog asset helpers — resolve the hub_url base and build absolute
// URLs for hub-hosted files (provider logos).
import { useEffect, useState } from "react";
import { api } from "../api/client";

/** Resolve a hub-relative path ("logos/foo.png") against the hub_url base,
    stripping the trailing catalog filename (e.g. "/catalog.json"). */
export function hubLogoUrl(hubUrl: string, logo: string): string {
  try {
    const u = new URL(hubUrl);
    const dir = u.pathname.replace(/\/[^/]*$/, "");
    return `${u.origin}${dir}/${logo}`;
  } catch {
    return logo;
  }
}

/** The configured hub_url (settings), fetched once per screen. Null until
    loaded, so consumers skip logo resolution until it's ready. */
export function useHubUrl(): string | null {
  const [hubUrl, setHubUrl] = useState<string | null>(null);
  useEffect(() => {
    let alive = true;
    api
      .getSettings()
      .then((s) => alive && setHubUrl(s.hub_url))
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, []);
  return hubUrl;
}
