// The session list, as the second view of the request-log card.
//
// Presentation only: the card owns the fetch, the window and the reload wiring,
// because the count and the window caption sit in the *same header row* as the
// tab switch — a child that fetched for itself would have to render them in a
// second row, and the toolbar would jump every time the reader switched tabs.
//
// The rows come from the daemon's merge of two ledgers (see `SessionVm`), and
// every rule below exists to keep that honest: the two sides are never summed,
// a zero on a side that never saw the session is not a measurement, and money is
// never converted outside the Dashboard.

import { useT } from "../../i18n";
import { fmtMoney, fmtTokens } from "../../lib/format";
import type { Session } from "../../api/types";

/** `2026-09-09T20:41:00Z` → `09-09 20:41` in the reader's own clock. */
function fmtStamp(ts: string): string {
  const at = new Date(ts);
  if (Number.isNaN(at.getTime())) return ts;
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${pad(at.getMonth() + 1)}-${pad(at.getDate())} ${pad(at.getHours())}:${pad(at.getMinutes())}`;
}

/** What one session's traffic cost, in the monies it cost them in.
 *
 * **Never summed across currencies and never converted.** A session whose
 * traffic spanned two providers has two facts, and adding them would be adding
 * dollars to yuan; the Dashboard is the one place a single figure exists. An
 * imported-only session has no cost at all rather than a zero — the files carry
 * tokens, not money, and a zero would read as "this was free". */
function fmtSessionCost(row: Session): string {
  if (row.cost.length === 0) {
    return row.unpriced_rows === 0 ? "—" : `${row.unpriced_rows} unpriced`;
  }
  const amounts = row.cost.map((c) => fmtMoney(c.amount, c.currency)).join(" + ");
  return row.unpriced_rows === 0 ? amounts : `${amounts} (+${row.unpriced_rows} unpriced)`;
}

/** Which ledger(s) this session was found in, and what that means for the row's
    numbers. The badge is load-bearing, not decoration: it is what tells a reader
    why `REQ` is `—` on some rows and `TURNS` is `—` on others. */
function SourceBadge({ source }: { source: Session["source"] }) {
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

export function SessionsTable({
  rows,
  loaded,
  onOpenSession,
}: {
  rows: Session[];
  loaded: boolean;
  /** A row click: narrow the request-log view to this conversation. */
  onOpenSession: (sessionId: string) => void;
}) {
  const t = useT();
  return (
    <div className="overflow-hidden">
      <table className="w-full text-[11.5px]">
        <thead>
          <tr className="border-b border-line text-left text-[10px] text-mut">
            <th className="px-3 py-2 font-medium">{t("logs.colTitle")}</th>
            <th className="px-2 py-2 font-medium">{t("logs.colSpan")}</th>
            <th className="px-2 py-2 font-medium">{t("logs.colAgent")}</th>
            <th className="px-2 py-2 font-medium">{t("logs.colProject")}</th>
            <th className="px-2 py-2 font-medium">{t("logs.colSource")}</th>
            <th className="px-2 py-2 text-right font-medium">{t("logs.colRequests")}</th>
            <th className="px-2 py-2 text-right font-medium">{t("logs.colTurns")}</th>
            <th className="px-2 py-2 text-right font-medium" title={t("logs.colTokensTitle")}>
              {t("logs.colTokens")}
            </th>
            <th className="px-2 py-2 text-right font-medium">{t("logs.colCost")}</th>
            <th className="px-3 py-2 font-medium">{t("logs.colSession")}</th>
          </tr>
        </thead>
        <tbody className="font-mono">
          {rows.map((r) => (
            <tr
              key={r.session_id}
              className="cursor-pointer border-b border-line hover:bg-surface2"
              title={t("logs.sessionRowTitle")}
              onClick={() => onOpenSession(r.session_id)}
            >
              {/* What this conversation was about. The agent's own title where
                  its file carries one, else the opening message — and `—` for a
                  session only the gateway saw, which has no file to ask. */}
              <td className="px-3 py-1.5 font-sans">
                <span
                  className={`block max-w-[260px] truncate${r.title ? "" : " text-mut"}`}
                  title={r.title ?? undefined}
                >
                  {r.title ?? "—"}
                </span>
              </td>
              <td className="whitespace-nowrap px-2 py-1.5 font-sans text-mut">
                {fmtStamp(r.started_at)} → {fmtStamp(r.ended_at)}
              </td>
              <td className="px-2 py-1.5 font-sans">{r.agent}</td>
              {/* The project is the files' dimension — the gateway sees requests,
                  not the checkout they came from — so a traffic-only row has
                  none, and `—` says "no side knew", not "none". */}
              <td
                className="max-w-[160px] truncate px-2 py-1.5 font-sans"
                title={r.machine ? `${t("logs.colMachine")}: ${r.machine}` : undefined}
              >
                {r.project ?? "—"}
              </td>
              <td className="px-2 py-1.5 font-sans">
                <SourceBadge source={r.source} />
              </td>
              {/* A turn in a file is not a request, and a request the files never
                  saw is not a turn. Whichever side is absent shows `—`. */}
              <td className="px-2 py-1.5 text-right">
                {r.source === "imported" ? "—" : r.requests}
              </td>
              <td className="px-2 py-1.5 text-right">{r.source === "gateway" ? "—" : r.turns}</td>
              <td className="whitespace-nowrap px-2 py-1.5 text-right">
                {fmtTokens(r.input_tokens)} / {fmtTokens(r.output_tokens)} /{" "}
                {fmtTokens(r.cache_read_tokens)} / {fmtTokens(r.cache_creation_tokens)}
              </td>
              <td className="whitespace-nowrap px-2 py-1.5 text-right">
                {fmtSessionCost(r)}
              </td>
              <td
                className="max-w-[140px] truncate px-3 py-1.5 text-mut"
                title={r.session_id}
              >
                {r.session_id}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      {loaded && rows.length === 0 && (
        <div className="px-3.5 py-6 text-center text-[11.5px] text-mut">
          {t("logs.sessionsEmpty")}
        </div>
      )}
    </div>
  );
}
