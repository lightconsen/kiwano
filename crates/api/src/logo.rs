//! The two rules that draw a provider's avatar: which letter, and which colour.
//!
//! Here rather than in the app for the reason `keys::mask_key` is: the daemon
//! produces these fields now (`logo_char` / `logo_color` on a route's binding
//! and on a provider row), and a second copy of the rule is how the two sides
//! would come to draw the same provider differently. `migrate.local.md` §10.7 —
//! the rule that produces a shape follows the shape.

const PALETTE: [&str; 6] = [
    "#4D6BFE", "#615CED", "#3859FF", "#F55036", "#6467F2", "#0F9D58",
];

/// One colour per name, stable across renders: the name's bytes pick the slot.
///
/// Deliberately not a random or most-recently-used choice — a provider that
/// changed colour between two screens would read as a different provider.
pub fn palette_color(name: &str) -> &'static str {
    let h: u64 = name.bytes().map(|b| (b as u64).wrapping_mul(31)).sum();
    PALETTE[(h as usize) % PALETTE.len()]
}

/// The letter in the avatar: the name's first character, uppercased. `?` for a
/// name with nothing in it, so the cell draws something rather than panicking.
pub fn logo_char(name: &str) -> String {
    name.chars()
        .next()
        .unwrap_or('?')
        .to_uppercase()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Stable, and inside the palette. Pinned because both sides render from
    /// this: a change here repaints every provider in the app.
    #[test]
    fn the_avatar_rules_are_stable_and_in_palette() {
        assert_eq!(logo_char("Alpha"), "A");
        assert_eq!(logo_char("deepseek"), "D");
        assert_eq!(logo_char(""), "?");
        // A multi-byte first character uppercases as a character, not a byte.
        assert_eq!(logo_char("中文"), "中");

        assert_eq!(palette_color("Alpha"), palette_color("Alpha"));
        assert!(PALETTE.contains(&palette_color("Alpha")));
    }
}
