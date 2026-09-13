// Time-of-day pricing, read the way the vendor writes it.
//
// The published shape: a price row's own rates are the **peak** ones and apply
// inside `peak_hours` — weekdays plus a `tz_offset` in minutes east of UTC —
// with `off_peak` in force outside them.
//
// This mirrors `crates/adapters/src/model_pricing.rs::is_peak`, which is the
// authority: the gateway bills with it, and this one only labels the shelf. If
// the two ever disagree, the gateway is right and the reader is paid what it
// says. Keep them structurally the same — half-open windows, a window that
// cannot be read never matches — because a divergence here is invisible.

export interface PeakWindow {
  days: string[];
  start: string;
  end: string;
}

export interface PeakHours {
  tz_offset: number;
  windows: PeakWindow[];
}

/** The weekday names the document uses, indexed as JS does (0 = Sunday). */
const DAYS = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];

/** Minutes since midnight for "HH:MM", or null when it is not a clock time. */
function minutesOfDay(hhmm: string): number | null {
  const [h, m] = hhmm.trim().split(":");
  const hours = Number(h);
  const minutes = Number(m);
  if (!Number.isInteger(hours) || !Number.isInteger(minutes)) return null;
  if (hours < 0 || hours > 23 || minutes < 0 || minutes > 59) return null;
  return hours * 60 + minutes;
}

/**
 * Is `at` inside a peak window? Read on the **vendor's** clock: judging these
 * windows against the reader's own timezone is the one mistake the offset
 * exists to prevent — it picks the wrong rate silently, by the difference
 * between the two zones.
 *
 * `at` is passed in rather than read from the clock here, so a caller can pin
 * it; the shelf takes `new Date()` once per render.
 */
export function isPeakAt(hours: PeakHours, at: Date): boolean {
  if (!hours.windows.length) return false;
  // Shift the instant so its UTC fields *are* the vendor's wall clock; then a
  // Day/weekday read is the vendor's day.
  const shifted = new Date(at.getTime() + hours.tz_offset * 60_000);
  const day = DAYS[shifted.getUTCDay()];
  const minutes = shifted.getUTCHours() * 60 + shifted.getUTCMinutes();

  return hours.windows.some((w) => {
    if (!w.days.some((d) => d.trim().toLowerCase() === day)) return false;
    const start = minutesOfDay(w.start);
    const end = minutesOfDay(w.end);
    if (start === null || end === null) return false;
    return start <= minutes && minutes < end;
  });
}

/** `+08:00` for 480 — the vendor's clock, shown wherever the windows are. */
export function tzLabel(offsetMinutes: number): string {
  const sign = offsetMinutes < 0 ? "-" : "+";
  const abs = Math.abs(offsetMinutes);
  const hh = String(Math.floor(abs / 60)).padStart(2, "0");
  const mm = String(abs % 60).padStart(2, "0");
  return `${sign}${hh}:${mm}`;
}
