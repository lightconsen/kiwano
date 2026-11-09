// The copy primitives the agent dialogs share: a value and the button that
// copies it, and one labelled line built from it.
import { useState } from "react";

import { Check, Copy } from "lucide-react";
import { Button } from "@/components/ui/button";
import { useT } from "../../i18n";
import { copyText } from "../../lib/clipboard";

/** A value and the button that copies it. The button is an icon: these are short
    values, and a word beside each of them competes with the thing being copied. */
export function CopyValue({ value }: { value: string }) {
  const t = useT();
  const [copied, setCopied] = useState(false);
  const copy = () => {
    // A copy that cannot happen must not take the row down with it: the tick
    // only appears when the clipboard actually took the value. (This used to go
    // through `navigator.clipboard` directly, which WKWebView does not provide —
    // so in the app the click did nothing at all. See `lib/clipboard.ts`.)
    void copyText(value).then((ok) => {
      if (!ok) return;
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1200);
    });
  };
  return (
    <>
      <code className="min-w-0 flex-1 truncate font-mono text-[11px]">{value}</code>
      <Button
        variant="ghost"
        size="sm"
        className="h-6 w-6 shrink-0 px-0 text-mut hover:text-ink"
        aria-label={copied ? t("common.copied") : t("common.copy")}
        title={copied ? t("common.copied") : t("common.copy")}
        onClick={copy}
      >
        {/* The tick is the confirmation: no layout shift, and the row keeps its
            width whether or not it was just used. */}
        {copied ? <Check className="h-3.5 w-3.5" /> : <Copy className="h-3.5 w-3.5" />}
      </Button>
    </>
  );
}

/** One line of the access card: a labelled value, with the copy button. */
export function CopyRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="mt-1.5 flex items-center gap-2">
      <span className="w-[68px] shrink-0 text-[10.5px] text-mut">{label}</span>
      <CopyValue value={value} />
    </div>
  );
}
