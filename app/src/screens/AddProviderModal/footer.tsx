// The dialog's two buttons: cancel, and the save that is refused while the form
// is not savable.
import { Button } from "@/components/ui/button";
import { DialogFooter } from "@/components/ui/dialog";
import { useT } from "../../i18n";
import type { Provider } from "../../api/types";

/** The footer. `save` is the container's, refused at its first line when
    `canSave` is false; the button says the same thing by being disabled. */
export function Footer({
  onClose,
  save,
  canSave,
  saving,
  edit,
}: {
  onClose: () => void;
  save: () => void;
  canSave: boolean;
  saving: boolean;
  edit: Provider | null;
}) {
  const t = useT();
  return (
    <DialogFooter className="mx-0 mb-0 flex-row justify-end gap-2 rounded-b-xl border-t border-line bg-transparent px-4 py-3">
      <Button variant="outline" size="sm" onClick={onClose}>
        {t("common.cancel")}
      </Button>
      <Button size="sm" className="font-semibold" disabled={!canSave} onClick={save}>
        {saving ? t("addProvider.saving") : edit ? t("common.save") : t("addProvider.saveEnable")}
      </Button>
    </DialogFooter>
  );
}
