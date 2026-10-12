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
      {/* `table-fixed`, and every column but two carries a width: the window's
          minimum is 900px, which leaves the card about 840px for nine columns,
          and a table that sized itself to its content would push the right-hand
          ones past the card's edge, where `overflow-hidden` **cuts** them rather
          than scrolling them. Fixed widths make the elastic columns truncate
          instead — which is what the titles are for — and the tooltips keep every
          value that a narrow window cannot show. */}
      <table className="w-full table-fixed text-[11.5px]">
        <colgroup>
          <col className="w-[34%]" />
          <col className="w-[76px]" />
          <col className="w-[46px]" />
          <col className="w-[13%]" />
          <col className="w-[54px]" />
          <col className="w-[38px]" />
          <col className="w-[42px]" />
          <col className="w-[84px]" />
          <col className="w-[86px]" />
        </colgroup>
        <thead>
          <tr className="border-b border-line text-left text-[10px] text-mut">
            <th className="px-3 py-2 font-medium">{t("logs.colTitle")}</th>
            <th className="px-1.5 py-2 font-medium" title={t("logs.colLastTitle")}>
              {t("logs.colLast")}
            </th>
            <th className="px-1.5 py-2 font-medium">{t("logs.colAgent")}</th>
            <th className="px-1.5 py-2 font-medium">{t("logs.colProject")}</th>
            <th className="px-1.5 py-2 font-medium">{t("logs.colSource")}</th>
            <th className="px-1.5 py-2 text-right font-medium">{t("logs.colRequests")}</th>
            <th className="px-1.5 py-2 text-right font-medium">{t("logs.colTurns")}</th>
            <th className="px-1.5 py-2 text-right font-medium" title={t("logs.colTokensTitle")}>
              {t("logs.colTokens")}
            </th>
            <th className="px-1.5 py-2 text-right font-medium">{t("logs.colCost")}</th>
          </tr>
        </thead>
        <tbody className="font-mono">
          {rows.map((r) => (
            <tr
              key={r.session_id}
              className="cursor-pointer border-b border-line hover:bg-surface2"
              // The session id lives here rather than in a column: at a 900px
              // window the card has room for nine columns and the id is the one a
              // reader never scans for — the click already takes them to that
              // session's requests, where the full id is on the dialog.
              title={t("logs.sessionRowTitle", { id: r.session_id })}
              onClick={() => onOpenSession(r.session_id)}
            >
              {/* What this conversation was about, **in full**: the agent's own
                  title where its file carries one, else the opening message — and
                  `—` for a session only the gateway saw, which has no file to ask.
                  A title is the one thing in this row a reader has to read rather
                  than compare, so it wraps instead of ellipsising: rows are as
                  tall as their titles, which is the price. `break-words` is not
                  decoration — a title can hold a URL, and an unbreakable word
                  wider than a fixed column would leave the cell rather than the
                  line. */}
              <td className="px-3 py-1.5 align-top font-sans">
                <span
                  className={`block break-words leading-snug${r.title ? "" : " text-mut"}`}
                >
                  {r.title ?? "—"}
                </span>
              </td>
              {/* When it was last active — the figure the list is ordered by,
                  and the one a reader scanning it is after. The start is a
                  separate question, and `--json` still carries it. */}
              <td className="whitespace-nowrap px-1.5 py-1.5 align-top font-sans text-mut">
                {fmtStamp(r.ended_at)}
              </td>
              <td className="truncate px-1.5 py-1.5 align-top font-sans" title={r.agent}>
                {r.agent}
              </td>
              {/* The project is the files' dimension — the gateway sees requests,
                  not the checkout they came from — so a traffic-only row has
                  none, and `—` says "no side knew", not "none". */}
              <td
                className="truncate px-1.5 py-1.5 align-top font-sans"
                title={[
                  r.project,
                  r.machine ? `${t("logs.colMachine")}: ${r.machine}` : null,
                ]
                  .filter(Boolean)
                  .join(" · ") || undefined}
              >
                {r.project ?? "—"}
              </td>
              <td className="px-1.5 py-1.5 align-top font-sans">
                <SourceBadge source={r.source} />
              </td>
              {/* A turn in a file is not a request, and a request the files never
                  saw is not a turn. Whichever side is absent shows `—`. */}
              <td className="px-1.5 py-1.5 text-right align-top">
                {r.source === "imported" ? "—" : r.requests}
              </td>
              <td className="px-1.5 py-1.5 text-right align-top">
                {r.source === "gateway" ? "—" : r.turns}
              </td>
              {/* Two of the four buckets, and the other two on hover: cache reads
                  dwarf everything else on a long conversation, and a list is for
                  comparing spend, not for reading four numbers per row. Nothing is
                  lost — the cell's title names all four, labelled. */}
              <td
                className="truncate px-1.5 py-1.5 text-right align-top"
                title={t("logs.tokensCellTitle", {
                  input: fmtTokens(r.input_tokens),
                  output: fmtTokens(r.output_tokens),
                  read: fmtTokens(r.cache_read_tokens),
                  write: fmtTokens(r.cache_creation_tokens),
                })}
              >
                {fmtTokens(r.input_tokens)} / {fmtTokens(r.output_tokens)}
              </td>
              <td
                className="truncate px-1.5 py-1.5 text-right align-top"
                title={fmtSessionCost(r)}
              >
                {fmtSessionCost(r)}
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
