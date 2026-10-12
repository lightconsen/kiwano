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
import { fmtTokens } from "../../lib/format";
import type { Session } from "../../api/types";
import { fmtSessionCost, fmtStamp, SourceBadge } from "./sessionBits";

export function SessionsTable({
  rows,
  loaded,
  onOpenSession,
  managedAgents,
}: {
  rows: Session[];
  loaded: boolean;
  /** The agents this install manages — empty on a machine that has taken none
   *  over, which is why the empty state below has two things to say. */
  managedAgents: string[];
  /** A row click: narrow the request-log view to this conversation. */
  onOpenSession: (sessionId: string) => void;
}) {
  const t = useT();
  return (
    <div className="overflow-hidden">
      {/* `table-fixed`, with a width for every column: the window's minimum is
          900px, which leaves this card about 860px for nine columns, and a table
          that sized itself to its content pushed the right-hand ones past the
          card's edge, where `overflow-hidden` **cuts** them rather than scrolling
          them.
          
          The widths are **measured**, not guessed — in a browser at 900px, against
          the widest value each column really holds (`deepseek-harness` for the
          agent id, `999k / 999k` for a token pair, `$2.5 + ¥7.25 (+3 unpriced)` for
          a two-currency cost). Two numbers with two currencies do not fit in this
          card and never will, so the columns that can wrap — title, project,
          agent, cost — **do** rather than truncating: a value that is cut is a
          different value, and a longer row is cheaper than a wrong one. */}
      <table className="w-full table-fixed text-[11.5px]">
        <colgroup>
          <col className="w-[27%]" />
          <col className="w-[84px]" />
          <col className="w-[64px]" />
          <col className="w-[10%]" />
          <col className="w-[54px]" />
          <col className="w-[44px]" />
          <col className="w-[46px]" />
          <col className="w-[104px]" />
          <col className="w-[110px]" />
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
              <td className="px-1.5 py-1.5 align-top font-sans">
                <span className="block break-words">{r.agent}</span>
              </td>
              {/* The project is the files' dimension — the gateway sees requests,
                  not the checkout they came from — so a traffic-only row has
                  none, and `—` says "no side knew", not "none". */}
              <td
                className="px-1.5 py-1.5 align-top font-sans"
                title={r.machine ? `${t("logs.colMachine")}: ${r.machine}` : undefined}
              >
                <span className="block break-words">{r.project ?? "—"}</span>
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
                className="whitespace-nowrap px-1.5 py-1.5 text-right align-top"
                title={t("logs.tokensCellTitle", {
                  input: fmtTokens(r.input_tokens),
                  output: fmtTokens(r.output_tokens),
                  read: fmtTokens(r.cache_read_tokens),
                  write: fmtTokens(r.cache_creation_tokens),
                })}
              >
                {fmtTokens(r.input_tokens)} / {fmtTokens(r.output_tokens)}
              </td>
              <td className="px-1.5 py-1.5 text-right align-top">
                <span className="block break-words">{fmtSessionCost(r, t)}</span>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      {loaded && rows.length === 0 && (
        <div className="px-3.5 py-6 text-center text-[11.5px] text-mut">
          {/* Two empties, two sentences. "Nothing in this window" and "nothing for
              the agents you manage" look identical and mean opposite things: the
              second is a list waiting for a takeover, and a reader who takes one
              agent over should not have to guess whether it worked. */}
          {managedAgents.length === 0 && !import.meta.env.DEV
            ? t("logs.sessionsEmptyUnmanaged")
            : t("logs.sessionsEmpty")}
        </div>
      )}
    </div>
  );
}
