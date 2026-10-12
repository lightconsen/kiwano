/** Local calendar dates ↔ the RFC3339 UTC instants the log filter compares.
 *
 * The bound has to be shaped like the values in the column, and those come from
 * Rust's `Utc::now().to_rfc3339()`: `2026-09-11T00:00:00+00:00`, sometimes with
 * a fractional part. `Date#toISOString()` produces
 * `2026-09-11T00:00:00.000Z` instead — and at index 19 `.` (0x2E) sorts *after*
 * `+` (0x2B), so a `.000Z` lower bound lands past a `+00:00` row at the very
 * same instant and silently drops it. Every bound this module emits is written
 * the way Rust writes timestamps, and `toBound` is the only place that decides
 * so.
 */

/** A Date → the RFC3339 UTC spelling Rust uses. Local midnights are always
 *  whole seconds, so the milliseconds are always exactly `.000`. */
function toBound(at: Date): string {
  return at.toISOString().replace(/\.\d{3}Z$/, "+00:00");
}

/** A Date → the `YYYY-MM-DD` an `<input type="date">` shows, in local time. */
function toDateInput(at: Date): string {
  return [
    at.getFullYear(),
    String(at.getMonth() + 1).padStart(2, "0"),
    String(at.getDate()).padStart(2, "0"),
  ].join("-");
}

/** `2026-09-11` → `2026-09-10T16:00:00+00:00` for a UTC+8 user: the instant
 *  that local day starts, as an inclusive `ts >=` bound. */
export function localMidnightUtc(date: string): string {
  const [y, m, d] = date.split("-").map(Number);
  return toBound(new Date(y, m - 1, d));
}

/** `2026-09-11` → the instant the *next* local day starts, as an exclusive
 *  `ts <` bound. Expressing the end as "next midnight, excluded" rather than
 *  "23:59:59, included" keeps the comparison exact — and `setDate(d + 1)`
 *  normalises month and year ends and lands on the true next local midnight
 *  whether the day is 23, 24 or 25 hours long. */
export function localMidnightUtcAfter(date: string): string {
  const [y, m, d] = date.split("-").map(Number);
  return toBound(new Date(y, m - 1, d + 1));
}

/** Today on the user's calendar. `toISOString().slice(0, 10)` would be the UTC
 *  date, which is yesterday for everyone east of UTC in their evening. */
export function todayLocalDate(): string {
  return toDateInput(new Date());
}

/** Shift a `YYYY-MM-DD` by whole days, in local time. */
export function shiftLocalDate(date: string, days: number): string {
  const [y, m, d] = date.split("-").map(Number);
  return toDateInput(new Date(y, m - 1, d + days));
}

/** The Dashboard's window, as the instant it starts — the same bound
 *  `crates/gateway/src/api/dashboard.rs::resolve_window` computes, so a list
 *  below those figures is measured over the same span they are.
 *
 * `today` is the local day, not the last 24 hours: the rolling reading is what
 * the daemon's `?days=N` means, and it disagrees with the dashboard at the day
 * boundary. `all` has no bound at all — the caller omits the parameter rather
 * than sending a zero, which would collapse the window to "now".
 *
 * `days - 1` because today counts as one of them: "7d" is today plus the six
 * before it, exactly as the daemon's window is. */
export function windowSince(win: "today" | "7d" | "30d" | "all"): string | undefined {
  const span = { today: 1, "7d": 7, "30d": 30 } as const;
  if (win === "all") return undefined;
  return localMidnightUtc(shiftLocalDate(todayLocalDate(), -(span[win] - 1)));
}
