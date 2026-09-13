// Time-of-day pricing, read the way the vendor writes it.
//
// The published shape: a price row's own rates are the **peak** ones and apply
// inside `peak_hours` — weekdays plus a `tz_offset` in minutes east of UTC —
// with `off_peak` in force outside them.
//
// Which of the two is in force at this moment is the gateway's answer, not this
// module's: `crates/adapters/src/model_pricing.rs::is_peak` decides it and bills
// with it, and the shelf shows both rates side by side rather than asserting
// that one of them is current. So what lives here is the vocabulary those
// labels need — the shape of a schedule, and the vendor's clock.

export interface PeakWindow {
  days: string[];
  start: string;
  end: string;
}

export interface PeakHours {
  tz_offset: number;
  windows: PeakWindow[];
}

/** `+08:00` for 480 — the vendor's clock, shown wherever the windows are. */
export function tzLabel(offsetMinutes: number): string {
  const sign = offsetMinutes < 0 ? "-" : "+";
  const abs = Math.abs(offsetMinutes);
  const hh = String(Math.floor(abs / 60)).padStart(2, "0");
  const mm = String(abs % 60).padStart(2, "0");
  return `${sign}${hh}:${mm}`;
}
