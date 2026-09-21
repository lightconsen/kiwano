// How the provider charges: the pills, or the label a catalog entry locked it to.
import { Label } from "@/components/ui/label";
import { useT } from "../../i18n";
import type { Billing, CatalogBilling, Provider } from "../../api/types";

/** The billing mode. A catalog entry states it — a vendor with several modes
    gets one entry per mode — so the pills are a label rather than a choice
    while an entry owns the form, and a stored row keeps the mode it was saved
    with. `both` is the one case where the user still picks. */
export function BillingSection({
  billing,
  setBilling,
  billOptions,
  billingKnown,
  billingLocked,
  bothEntry,
  unresolvedBoth,
  edit,
}: {
  billing: CatalogBilling;
  setBilling: (b: CatalogBilling) => void;
  /** The three modes, each with the label the translator built for it */
  billOptions: { id: Billing; label: string }[];
  billingKnown: boolean;
  billingLocked: boolean;
  bothEntry: boolean;
  unresolvedBoth: boolean;
  edit: Provider | null;
}) {
  const t = useT();
  return (
    <div>
      <Label className="text-[11px] font-medium text-mut">
        {t("addProvider.billing")}
        {billingLocked && (
          <span className="ml-1 text-[10px]" style={{ color: "var(--kiwi)" }}>
            {edit ? t("addProvider.billingFixed") : t("addProvider.billingFromCatalog")}
          </span>
        )}
      </Label>
      {billingLocked ? (
        <div className="mt-1 flex h-8 items-center rounded-md border border-line bg-surface2 px-2.5 text-[11.5px] text-ink">
          {billOptions.find((b) => b.id === billing)?.label ?? billing}
        </div>
      ) : (
        <div className={`mt-1 grid gap-1.5 ${bothEntry ? "grid-cols-2" : "grid-cols-3"}`}>
          {(bothEntry ? billOptions.filter((b) => b.id !== "unl") : billOptions).map((b) => {
            const active = billing === b.id;
            return (
              <button
                key={b.id}
                className="btn h-8 cursor-pointer rounded-md border text-[11.5px]"
                style={active
                  ? { borderColor: "var(--kiwi-dim)", background: "var(--kiwi-soft)", color: "var(--kiwi)" }
                  : { borderColor: "var(--line)", color: "var(--mut)" }}
                onClick={() => setBilling(b.id)}
              >
                {b.label}
              </button>
            );
          })}
        </div>
      )}
      {!billingKnown && (
        <p className="mt-1 text-[10.5px]" style={{ color: "var(--amber)" }}>
          {t("addProvider.billingUnknown", { billing })}
        </p>
      )}
      {bothEntry && (
        // Amber while the choice is still owed, muted once it is made:
        // same sentence either way, because the fact it states — this
        // vendor charges both — outlives the decision.
        <p
          className="mt-1 text-[10.5px]"
          style={{ color: unresolvedBoth ? "var(--amber)" : "var(--mut)" }}
        >
          {t("addProvider.billingBoth")}
        </p>
      )}
    </div>
  );
}
