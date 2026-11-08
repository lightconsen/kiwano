import { describe, expect, it } from "vitest";

import { fmtBytes } from "./format";

describe("fmtBytes", () => {
  it("picks the unit the size is worth reading in", () => {
    expect(fmtBytes(0)).toBe("0 B");
    expect(fmtBytes(999)).toBe("999 B");
    expect(fmtBytes(1_500)).toBe("1.5 KB");
    // One decimal below ten, whole numbers above it: the fraction is noise once
    // the file is hundreds of megabytes.
    expect(fmtBytes(9_876_543)).toBe("9.9 MB");
    expect(fmtBytes(670_300_000)).toBe("670 MB");
    expect(fmtBytes(9_400_000_000)).toBe("9.4 GB");
  });
});
