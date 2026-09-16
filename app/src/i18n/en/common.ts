// Strings shared across screens. Anything used by more than one namespace
// belongs here rather than being duplicated — two copies of "Cancel" drift.
export const common = {
  /** The language select's `system` option. Named in the UI's own language,
      because "跟随系统" is no help to a reader whose UI is English. */
  languageSystem: "System",
  language: "Language",

  cancel: "Cancel",
  save: "Save",
  close: "Close",
  done: "Done",
  edit: "Edit",
  remove: "Remove",
  delete: "Delete",
  add: "Add",
  refresh: "Refresh",
  retry: "Retry",
  /** A read a screen could not make. The raw failure rides in the tooltip — it
      is rarely a sentence, and this is. */
  loadFailed: "Couldn't read this screen's data",
  loading: "Loading…",
  none: "—",
  enabled: "Enabled",
  disabled: "Disabled",
  copy: "Copy",
  copied: "Copied",
};
