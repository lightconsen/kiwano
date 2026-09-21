// The advanced forwarding settings a provider can carry.

/** The headers the gateway injects as credentials (`x-api-key` / `Authorization`
    / `x-goog-api-key`, see `gateway::forward`). A custom header with one of
    these names wins over the injected credential — which is sometimes exactly
    the point (an Azure-style `api-key` endpoint), and otherwise a silent way to
    send the wrong key upstream. Warned about, not refused: this dialog cannot
    know which it is. */
export const AUTH_HEADER_NAMES = ["authorization", "x-api-key", "x-goog-api-key"];
