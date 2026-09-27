// Hub contract (docs/hub-api.md, v67): a catalog endpoint whose URL carries
// {placeholder} segments is a template — the user fills the values when
// adding the provider, and the composed URL is what gets stored. Templates
// exist only at add time; a saved provider always holds a concrete endpoint,
// so nothing downstream (gateway, metering, edit) ever sees a placeholder.
//
// The placeholder syntax is the Hub's: `{name}` with a lowercase name of
// letters, digits and dashes (region, resource-name, account-id, …). The
// parsers here are the frontend's only copy of that rule — the core has no
// counterpart, because instantiation happens in this dialog and the backend
// only ever receives the composed URL.

const PLACEHOLDER = /\{([a-z][a-z0-9-]*)\}/g;

/** The placeholder names a template URL carries, in order of appearance. */
export function endpointParams(template: string): string[] {
  return [...template.matchAll(PLACEHOLDER)].map((m) => m[1]);
}

export function isTemplateEndpoint(url: string): boolean {
  return endpointParams(url).length > 0;
}

/** A template whose parameters are not all filled yet — it still carries a
    visible hole, so probing, model-fetching and saving are all off until the
    user completes it. */
export function hasUnfilledParams(url: string, params: Record<string, string>): boolean {
  return endpointParams(url).some((name) => !(params[name] ?? "").trim());
}

/** Substitute the filled parameters into the template. Unfilled holes stay
    as `{name}` — the preview shows them, and the save gate refuses them. */
export function instantiateEndpoint(
  template: string,
  params: Record<string, string>,
): string {
  return template.replace(PLACEHOLDER, (raw, name) => {
    const v = params[name]?.trim();
    return v ? v : raw;
  });
}
