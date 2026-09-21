// The endpoint rows the form collects, and the probe readout beside each one.
import type { ProbeReport } from "../../api/types";

export function probeColor(verdict: ProbeReport["verdict"]): string {
  return verdict === "ok" || verdict === "auth" ? "var(--kiwi)" : "var(--red)";
}
