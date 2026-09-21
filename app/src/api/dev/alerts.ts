// The sample alerts' once-per-session dedup, mirroring the per-period dedup
// `vm::check_usage_alerts` keeps. The alert list itself is `checkUsageAlerts`
// in `api/alerts.ts`.

/** Dev-only dedup for the sample feature alerts (see checkUsageAlerts). */
export const devAlerted = new Set<string>();
