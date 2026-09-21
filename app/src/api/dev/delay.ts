// The mock's latency: every method waits on it, so the loading states a real
// round trip produces are reachable in `pnpm dev`.

export async function delay(ms = 120) {
  return new Promise((r) => setTimeout(r, ms));
}
