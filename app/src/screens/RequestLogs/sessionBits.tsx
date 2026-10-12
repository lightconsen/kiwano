// The session vocabulary, shared by the two views that speak it: the list's
// Sessions tab and the detail page a row opens.
//
// Extracted rather than exported from `SessionsTable` because the dependency
// would run the wrong way — a page importing from the table that links to it.
// Everything here is presentation, and each piece carries a rule the project
// holds elsewhere, so the two views cannot come to disagree about them.

import { useT, type Translate } from "../../i18n";
import { fmtMoney } from "../../lib/format";
import type { Session } from "../../api/types";

/** `2026-09-09T20:41:00Z` → `09-09 20:41` in the reader's own clock.
 *
 * The list's stamp. A detail page wants the year, so it formats its own — this
 * one is short by design, because a column of full ISO stamps is a column nobody
 * scans. */
export function fmtStamp(ts: string): string {
  const at = new Date(ts);
  if (Number.isNaN(at.getTime())) return ts;
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${pad(at.getMonth() + 1)}-${pad(at.getDate())} ${pad(at.getHours())}:${pad(at.getMinutes())}`;
}

/** A full local timestamp, for a page with room for the year. */
export function fmtFullStamp(ts: string, locale: string): string {
  const at = new Date(ts);
  if (Number.isNaN(at.getTime())) return ts;
  return at.toLocaleString(locale, { dateStyle: "medium", timeStyle: "short" });
}

/** What one session's traffic cost, in the monies it cost them in.
 *
 * **Never summed across currencies and never converted.** A session whose
 * traffic spanned two providers has two facts, and adding them would be adding
 * dollars to yuan; the Dashboard is the one place a single figure exists, and it
 * gets there on the backend. An imported-only session has no cost at all rather
 * than a zero — the files carry tokens, not money, and a zero would read as
 * "this was free". */
export function fmtSessionCost(row: Session, t: Translate): string {
  if (row.cost.length === 0) {
    return row.unpriced_rows === 0
      ? t("common.none")
      : t("logs.unpricedRows", { n: row.unpriced_rows });
  }
  const amounts = row.cost.map((c) => fmtMoney(c.amount, c.currency)).join(" + ");
  return row.unpriced_rows === 0
    ? amounts
    : t("logs.costPlusUnpriced", { amounts, n: row.unpriced_rows });
}

/** Which ledger(s) this session was found in, and what that means for the row's
    numbers. The badge is load-bearing, not decoration: it is what tells a reader
    why `REQ` is `—` on some rows and `TURNS` is `—` on others. */
export function SourceBadge({ source }: { source: Session["source"] }) {
  const t = useT();
  // The daemon writes one of three strings; an unknown one still has to render
  // as something (the wire type is a plain string), so it falls back to itself.
  const label =
    source === "both"
      ? t("logs.sourceBoth")
      : source === "imported"
        ? t("logs.sourceImported")
        : source === "gateway"
          ? t("logs.sourceGateway")
          : source;
  const tone =
    source === "both" ? "var(--kiwi)" : source === "imported" ? "var(--violet)" : "var(--mut)";
  return (
    <span className="text-[10px]" style={{ color: tone }} title={t("logs.sourceTitle")}>
      {label}
    </span>
  );
}
