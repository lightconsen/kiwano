//! Presentation helpers: the chart palette, and the token formatter the CLI and
//! the dashboard both print.
//!
//! The avatar rules (`logo_char`, `palette_color`) are `kiwano_api::logo`'s, and
//! the provider view that used them is the daemon's now (`migrate.local.md`
//! §10.21) — so nothing in this crate reads them and there is nothing to
//! re-export.

/// Token formatting, mirroring `src/lib/format.ts`.
pub fn fmt_tokens(v: i64) -> String {
    if v >= 1_000_000 {
        let m = v as f64 / 1_000_000.0;
        if m >= 10.0 {
            format!("{}M", m.round() as i64)
        } else {
            format!("{}M", (m * 10.0).round() / 10.0)
        }
    } else if v >= 1_000 {
        format!("{}k", (v as f64 / 1_000.0).round() as i64)
    } else {
        v.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kiwanod::api::views::chart_palette;

    #[test]
    fn fmt_tokens_matches_frontend() {
        assert_eq!(fmt_tokens(6_200_000), "6.2M");
        assert_eq!(fmt_tokens(8_800_000), "8.8M");
        assert_eq!(fmt_tokens(200_000), "200k");
        assert_eq!(fmt_tokens(31), "31");
        assert_eq!(fmt_tokens(15_000_000), "15M");
    }

    #[test]
    fn chart_palette_keeps_slices_distinct() {
        // More names than colours: a taken slot steps to the next free one, so
        // the slices stay distinguishable rather than sharing a hash.
        let ids: Vec<String> = (0..8).map(|i| format!("provider-{i}")).collect();
        let colors = chart_palette(&ids);
        let unique: std::collections::HashSet<_> = colors.iter().collect();
        assert_eq!(unique.len(), colors.len(), "each slice gets its own colour");

        // Stable: the same roster yields the same colours on every render.
        assert_eq!(chart_palette(&ids), colors);
    }
}
