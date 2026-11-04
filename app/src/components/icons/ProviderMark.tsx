// A provider row's mark, in the order the sources are authoritative:
//
//   1. the logo the Hub publishes for the catalog entry the row is linked to —
//      an actual brand asset, resolved by the backend against `hub_url`;
//   2. a mark inferred from the endpoint host (see ./infer), for a provider the
//      catalog does not list — a self-hosted relay, a vendor added by hand;
//   3. the letter avatar, when nothing names the brand at all.
//
// This is one component rather than the same ternary at every call site because
// the order is the point: forgetting step 1 is what put a letter avatar on a
// Command Code provider whose logo the catalog had all along.
import { ProviderLogo } from "./ProviderLogo";
import { iconForEndpoint } from "./infer";

export function ProviderMark({
  provider,
  size,
  className = "",
}: {
  provider: { name: string; endpoint: string; logo_char: string; logo_color: string; catalog_logo?: string | null };
  size: number;
  className?: string;
}) {
  return (
    <ProviderLogo
      logo={provider.catalog_logo}
      icon={iconForEndpoint(provider.endpoint)}
      char={provider.logo_char}
      color={provider.logo_color}
      name={provider.name}
      size={size}
      className={className}
    />
  );
}
