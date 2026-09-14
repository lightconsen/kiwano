// The vendor's clock, as text.
//
// `lib/peak.ts` is deliberately only the vocabulary — which rate is in force at
// this moment is the gateway's answer (`model_pricing.rs::is_peak`), and the
// shelf shows both rates side by side rather than asserting one of them. What is
// left here is a label, and a label that is wrong by a sign or a zero-pad sends
// the reader to the wrong timezone.
import { describe, expect, it } from "vitest";

import { tzLabel } from "./peak";

describe("the vendor's clock label", () => {
  it("writes an offset in minutes as a UTC offset", () => {
    expect(tzLabel(480)).toBe("+08:00"); // Beijing
    expect(tzLabel(0)).toBe("+00:00"); // UTC itself, not "-00:00"
    expect(tzLabel(-300)).toBe("-05:00"); // and the sign survives
  });

  it("keeps the minutes of a half-hour zone", () => {
    expect(tzLabel(330)).toBe("+05:30"); // India
    expect(tzLabel(-210)).toBe("-03:30"); // Newfoundland
    expect(tzLabel(45)).toBe("+00:45"); // and a quarter-hour one still pads
  });
});
