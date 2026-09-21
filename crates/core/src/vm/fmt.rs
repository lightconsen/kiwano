//! Presentation helpers: the palette a slice or a logo gets, and the token
//! formatter the CLI and the dashboard both print.

const PALETTE: [&str; 6] = [
    "#4D6BFE", "#615CED", "#3859FF", "#F55036", "#6467F2", "#0F9D58",
];

pub(crate) fn palette_color(name: &str) -> &'static str {
    let h: u64 = name.bytes().map(|b| (b as u64).wrapping_mul(31)).sum();
    PALETTE[(h as usize) % PALETTE.len()]
}

/// Categorical colours for charts. Spread around the hue wheel and held at a
/// lightness that reads on both themes — the letter-avatar palette above is
/// blue-heavy, which is fine behind a white glyph and useless for slices that
/// have to be told apart.
const CHART_COLORS: [&str; 8] = [
    "#4D6BFE", // blue
    "#0F9D58", // green
    "#F55036", // orange-red
    "#9333EA", // purple
    "#0EA5E9", // cyan
    "#EAB308", // amber
    "#EC4899", // pink
    "#14B8A6", // teal
];

/// One colour per id, distinct within the list: the id picks the starting slot
/// (so a provider keeps its colour while the roster holds still) and a taken
/// slot steps to the next free one. Only past eight entries do colours repeat.
pub(crate) fn chart_palette(ids: &[String]) -> Vec<&'static str> {
    let mut taken = [false; CHART_COLORS.len()];
    ids.iter()
        .map(|id| {
            let h: u64 = id.bytes().map(|b| (b as u64).wrapping_mul(31)).sum();
            let mut i = (h as usize) % CHART_COLORS.len();
            for _ in 0..CHART_COLORS.len() {
                if !taken[i] {
                    break;
                }
                i = (i + 1) % CHART_COLORS.len();
            }
            taken[i] = true;
            CHART_COLORS[i]
        })
        .collect()
}

pub(crate) fn logo_char(name: &str) -> String {
    name.chars()
        .next()
        .unwrap_or('?')
        .to_uppercase()
        .to_string()
}

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
