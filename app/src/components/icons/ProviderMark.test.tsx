// The order of a provider row's mark: the catalog's logo, then the endpoint's
// brand, then the letter. Each step is what the one before it could not name,
// and a Command Code provider is the reason the order is asserted — the catalog
// published its logo while the row drew a "C" because only the endpoint was
// consulted.
import { describe, expect, it } from "vitest";
import { render } from "@testing-library/react";
import { ProviderMark } from "./ProviderMark";

const base = {
  name: "Command Code",
  endpoint: "https://api.commandcode.ai/provider/v1",
  logo_char: "C",
  logo_color: "#111827",
};

describe("ProviderMark", () => {
  it("draws the catalog's logo when the row is linked to an entry", () => {
    const { container } = render(
      <ProviderMark
        provider={{ ...base, catalog_logo: "https://hub.example/logos/commandcode.png" }}
        size={32}
      />,
    );
    const img = container.querySelector("img");
    expect(img?.getAttribute("src")).toBe("https://hub.example/logos/commandcode.png");
    // …and not the letter avatar that the earlier code fell back to.
    expect(container.textContent).toBe("");
  });

  it("infers the vendor from the endpoint when the catalog knows nothing", () => {
    // No `catalog_logo`: this is a provider the Hub does not list, and the
    // endpoint still names the brand.
    const { container } = render(<ProviderMark provider={base} size={32} />);
    const mark = container.querySelector('[aria-label="Command Code"]');
    expect(mark?.querySelector("svg")).not.toBeNull();
  });

  it("falls back to the letter when neither source names the brand", () => {
    const { container } = render(
      <ProviderMark provider={{ ...base, endpoint: "https://relay.internal" }} size={32} />,
    );
    expect(container.textContent).toBe("C");
  });
});
