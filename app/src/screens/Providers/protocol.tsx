// A protocol's name in the reader's language, and the picker that offers the
// ones Kiwano can serve.
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useT, type Translate } from "../../i18n";
import { type Protocol } from "../../api/types";

/** A protocol's name in the reader's language — the three words the add-provider
    dialog already uses for a provider's own protocol. */
export function protocolName(t: Translate, p: Protocol): string {
  if (p === "openai") return t("addProvider.protoOpenai");
  if (p === "anthropic") return t("addProvider.protoAnthropic");
  return t("addProvider.protoGemini");
}

/** `null` travels as this through the picker: Radix reserves the empty string
    for "no value at all", and "not said" is a value here — the state of a row
    defined before the field existed, which must survive a save untouched. */
const PROTOCOL_UNSET = "__unset__";

/** Which wire format an agent's own clients speak.
 *
 * A **label**, in both directions: nothing routes, validates or filters by it
 * (the gateway reads an inbound's protocol from the path it was called on), and
 * the picker offers no protocol that Kiwano cannot serve. So the only mistake
 * on offer here is leaving it unsaid; a wrong one is a note that misleads, not
 * a request that fails. */
export function AgentProtocolSelect({
  value,
  onChange,
  allowUnset,
  className,
}: {
  value: Protocol | null;
  onChange: (p: Protocol | null) => void;
  /** Offer "not specified" as a destination. A new agent is asked the question
      outright and answers it; an existing one may already be in that state and
      has to be able to stay there. */
  allowUnset?: boolean;
  className?: string;
}) {
  const t = useT();
  const label = (v: string) =>
    v === PROTOCOL_UNSET ? t("providers.agentProtocolUnset") : protocolName(t, v as Protocol);
  return (
    <Select
      value={value ?? PROTOCOL_UNSET}
      onValueChange={(v) => v && onChange(v === PROTOCOL_UNSET ? null : (v as Protocol))}
    >
      <SelectTrigger className={className} aria-label={t("providers.agentProtocol")}>
        <SelectValue>{(v) => label(String(v ?? value ?? PROTOCOL_UNSET))}</SelectValue>
      </SelectTrigger>
      <SelectContent>
        {allowUnset && (
          <SelectItem value={PROTOCOL_UNSET}>{t("providers.agentProtocolUnset")}</SelectItem>
        )}
        {(["openai", "anthropic", "gemini"] as Protocol[]).map((p) => (
          <SelectItem key={p} value={p}>
            {protocolName(t, p)}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}
