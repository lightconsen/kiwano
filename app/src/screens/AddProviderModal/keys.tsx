// The rotating keys a provider answers with, and the box that adds one.
import { Plus, XIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useT } from "../../i18n";
import type { ApiKeyEntry, UpstreamKeySpendEntry } from "../../api/types";
import { fmtMoney } from "../../lib/format";

/** The name the ledger gives a provider's own key (see `PoolHandle`). */
const PRIMARY = "primary";

/** What the pool's spend rows say about one entry.
 *
 * The backend records the entry by the name a person gave it — a label, else
 * `#<id>` — so a row's own spend is looked up by first reproducing that name
 * exactly as the backend derives it. Matching on the label rather than the row
 * id is what keeps two keys that share an account summed together, which is
 * what the reader means by "what did this account cost". */
export function spendForKey(spend: UpstreamKeySpendEntry[], key: ApiKeyEntry): UpstreamKeySpendEntry[] {
  const name = key.label?.trim() || `#${key.id}`;
  return spend.filter((s) => s.upstream_key === name);
}

/** One entry's spend, in the monies it spent them in.
 *
 * **Never summed across currencies and never converted** — this is not the
 * Dashboard, and a single figure here would be either a conversion or a lie.
 * Rows that could not be priced are named rather than folded in: a total that is
 * quietly short is worse than one that says it is short. */
function KeySpend({ spend, days }: { spend: UpstreamKeySpendEntry[]; days: number }) {
  const t = useT();
  const requests = spend.reduce((n, s) => n + s.requests, 0);
  if (requests === 0) return null;
  const unpriced = spend.reduce((n, s) => n + s.unpriced_rows, 0);
  const amounts = spend
    .filter((s) => s.currency)
    .map((s) => fmtMoney(s.cost, s.currency as string));
  const money =
    amounts.length === 0
      ? t("addProvider.keySpendUnpriced", { rows: requests })
      : unpriced > 0
        ? t("addProvider.keySpendPartial", { amounts: amounts.join(" + "), rows: unpriced })
        : amounts.join(" + ");
  return (
    <span className="text-[10.5px] text-mut" title={t("addProvider.keySpendTitle", { days })}>
      {money}
    </span>
  );
}

/** Edit mode only: several keys on one provider rotate automatically. The two
    calls behind the buttons are the container's. */
export function RotatingKeys({
  pollKeys,
  spend,
  spendDays,
  newKey,
  setNewKey,
  newKeyLabel,
  setNewKeyLabel,
  keyBusy,
  addPollKey,
  removePollKey,
}: {
  pollKeys: ApiKeyEntry[];
  /** What each entry of the pool spent over `spendDays` — see `spendText`. */
  spend: UpstreamKeySpendEntry[];
  spendDays: number;
  newKey: string;
  setNewKey: (v: string) => void;
  newKeyLabel: string;
  setNewKeyLabel: (v: string) => void;
  keyBusy: boolean;
  addPollKey: () => void;
  removePollKey: (id: number) => void;
}) {
  const t = useT();
  return (
    <div>
      <Label className="text-[11px] font-medium text-mut">
        {t("addProvider.rotatingKeys")}{" "}
        <span className="ml-1 text-[10px]" style={{ color: "var(--kiwi)" }}>
          {t("addProvider.rotatingKeysHint")}
        </span>
      </Label>
      <div className="mt-1 space-y-1">
        {pollKeys.map((k) => (
          <div
            key={k.id}
            className="flex h-8 items-center justify-between rounded-md border border-line px-2.5"
          >
            <span className="flex min-w-0 items-center gap-2">
              <span className="font-mono text-[11.5px]">{k.masked}</span>
              {k.label && <span className="text-[10.5px] text-mut">{k.label}</span>}
            </span>
            <span className="flex items-center gap-2">
              <KeySpend spend={spendForKey(spend, k)} days={spendDays} />
              <Button
                variant="ghost"
                size="icon-xs"
                className="text-mut"
                aria-label={t("addProvider.deleteKey")}
                onClick={() => removePollKey(k.id)}
              >
                <XIcon />
              </Button>
            </span>
          </div>
        ))}
        {/* The primary key has no row to sit on — it lives on the provider, not
            in the pool — but it is a pool entry like any other and its spend
            belongs on this list rather than in a total the reader has to
            subtract. Shown only when it has spent something: a row for a key
            that has never carried a request is a line about nothing. */}
        {spend.filter((s) => s.upstream_key === PRIMARY).length > 0 && (
          <div className="flex h-8 items-center justify-between rounded-md border border-line border-dashed px-2.5">
            <span className="text-[11.5px] text-mut">{t("addProvider.primaryKey")}</span>
            <KeySpend spend={spend.filter((s) => s.upstream_key === PRIMARY)} days={spendDays} />
          </div>
        )}
        <div className="flex gap-1.5">
          <Input
            className="h-8 min-w-0 flex-1 bg-bg font-mono text-[12px] dark:bg-bg"
            value={newKey}
            onChange={(e) => setNewKey(e.target.value)}
            placeholder={t("addProvider.addKeyPlaceholder")}
          />
          <Input
            className="h-8 w-[84px] bg-bg text-[11.5px] dark:bg-bg"
            value={newKeyLabel}
            onChange={(e) => setNewKeyLabel(e.target.value)}
            placeholder={t("addProvider.labelPlaceholder")}
          />
          <Button
            variant="outline"
            size="sm"
            className="h-8 gap-1 px-2.5 text-[11px]"
            disabled={!newKey.trim() || keyBusy}
            onClick={addPollKey}
          >
            <Plus className="h-3 w-3" />
            {t("common.add")}
          </Button>
        </div>
      </div>
    </div>
  );
}
