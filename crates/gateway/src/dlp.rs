//! Outbound credential detection: what an agent is about to send that it
//! probably should not.
//!
//! This is a **detector**, and the one next door in [`crate::log_capture`] is
//! not. That module removes values it already knows — the store's own provider
//! keys by value, anything under a [`SECRET_KEYS`](crate::log_capture) name, and
//! anything carrying a listed prefix — so a credential it has never seen, in a
//! shape nobody listed, reaches the provider untouched. This pass looks for the
//! shapes themselves and says so.
//!
//! **It reports; it does not block.** By the time a finding exists the payload
//! has already been forwarded, so the honest description of this feature is
//! "you find out it left", not "it did not leave". Nothing here waits, and
//! nothing here can fail a request.
//!
//! Three boundaries worth stating rather than discovering:
//!
//! * **Literal anchors only.** Every rule is a vendor prefix, a JWT's `eyJ`
//!   head, or a PEM header. A credential in a format nobody listed passes.
//!   High-entropy guessing is deliberately absent: `token_run_len` is only ever
//!   a *tail condition* on a literal prefix today, and using it to flag bare
//!   long runs would fire on base64 blobs, long identifiers and hashes — which
//!   is the noise that kills a detector that is on by default.
//! * **Encoded exfiltration is invisible.** A payload base64'd or encrypted
//!   after the fact matches no anchor here.
//! * **Only what goes through the gateway.** An agent that connects straight to
//!   a provider is not seen at all.
//!
//! Findings carry the rule and a count and **never the matched text**. The
//! gateway already stores the body; a scanner that wrote what it caught into
//! the log as well would be a second copy of the thing it is warning about.

/// One rule that fired, and how many times in this payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub rule: &'static str,
    pub count: usize,
}

/// The credential shapes this pass reports on.
///
/// Deliberately **not** `log_capture::TOKEN_PREFIXES`, though the two overlap
/// heavily. That table exists to *remove* a value before it is stored; this one
/// exists to *report* that a value is leaving. A rule that belongs in one does
/// not always belong in the other, and merging them would change what the
/// request log redacts — a security-relevant behaviour with its own tests.
///
/// Two entries from that table are deliberately absent:
///
/// * `kw-ag-` — Kiwano's own placeholder key. It is in the config of every
///   agent a takeover has rewritten, so reporting it would fire on nearly every
///   request. Redacting it is still right; that is the whole asymmetry.
/// * `Bearer ` — an authorization scheme, not a credential, and it appears in
///   prose constantly. The token after it is caught by its own shape.
///
/// `(prefix, rule, min_tail)` — `min_tail` is how many token bytes must follow
/// before it counts, which is what keeps `sk-` in a sentence from matching.
const PREFIX_RULES: &[(&str, &str, usize)] = &[
    // Anthropic before the bare `sk-`, so the more specific prefix wins.
    ("sk-ant-", "anthropic-key", 20),
    ("sk-proj-", "openai-key", 20),
    ("sk-", "openai-key", 20),
    ("sk_live_", "stripe-key", 20),
    ("rk_live_", "stripe-key", 20),
    ("github_pat_", "github-token", 20),
    ("ghp_", "github-token", 20),
    ("gho_", "github-token", 20),
    ("ghu_", "github-token", 20),
    ("ghs_", "github-token", 20),
    ("ghr_", "github-token", 20),
    ("glpat-", "gitlab-token", 20),
    ("xoxb-", "slack-token", 20),
    ("xoxp-", "slack-token", 20),
    ("xoxa-", "slack-token", 20),
    ("xoxr-", "slack-token", 20),
    ("xoxs-", "slack-token", 20),
    ("AKIA", "aws-access-key", 16),
    ("npm_", "npm-token", 20),
    ("pypi-", "pypi-token", 20),
    ("dckr_pat_", "docker-token", 20),
    ("AIza", "google-api-key", 30),
];

/// What a PEM private key block is announced by. `RSA`, `EC`, `OPENSSH` and the
/// bare form all end with these same bytes, so one literal covers them.
const PEM_MARKER: &str = "PRIVATE KEY-----";

/// The opening marker. Between it and [`PEM_MARKER`] there is at most a type
/// word, which is what the check below allows for — `BEGIN` alone would reject
/// `-----BEGIN RSA PRIVATE KEY-----`, and a bare `PRIVATE KEY-----` would match
/// the marker quoted in prose.
const PEM_HEAD: &str = "-----BEGIN ";

/// How long a type word may be (`OPENSSH` is the longest in common use).
const PEM_TYPE_MAX: usize = 16;

/// At most this many rule lines reach the note: a pathological payload must not
/// be able to write a kilobyte of prose into a log column. Same spirit as the
/// shim's own cap, and the overflow is named rather than dropped silently.
const MAX_RULES: usize = 5;

/// Scan an outbound request body, returning what fired and how often.
///
/// `cap` mirrors the capture's own `max_body_bytes`: findings cover the part of
/// the body the log would keep, and nothing beyond it. The default is no cap,
/// so this is only a real bound for someone who set one.
pub fn scan(bytes: &[u8], cap: Option<usize>) -> Vec<Finding> {
    let capped = match cap {
        Some(max) => &bytes[..bytes.len().min(max)],
        None => bytes,
    };
    // Lossy: a body that is not UTF-8 still has its ASCII prefixes in the
    // surviving text, which is all an anchor needs.
    let text = String::from_utf8_lossy(capped);
    let raw = text.as_bytes();
    let mut counts: Vec<Finding> = Vec::new();

    // Positions a JWT occupied, so its interior is not rescanned.
    let mut skip_until = 0usize;
    for (i, _) in text.char_indices() {
        if i < skip_until || (i > 0 && crate::log_capture::is_token_byte(raw[i - 1])) {
            // Inside a token already matched, or mid-word: a match has to start
            // a token, so `task-sk-…` is a word rather than a key.
            continue;
        }
        let rest = &text[i..];
        if let Some(len) = crate::log_capture::jwt_len(rest) {
            bump(&mut counts, "jwt");
            skip_until = i + len;
            continue;
        }
        let hit = PREFIX_RULES.iter().find(|(prefix, _, min_tail)| {
            rest.starts_with(prefix)
                && crate::log_capture::token_run_len(&raw[i + prefix.len()..]) >= *min_tail
        });
        if let Some((prefix, rule, min_tail)) = hit {
            bump(&mut counts, rule);
            skip_until = i + prefix.len() + min_tail;
        }
    }

    // Private keys, by their header line rather than by a token run.
    let mut from = 0usize;
    while let Some(at) = text[from..].find(PEM_MARKER) {
        let end = from + at;
        // Walk back to the opening marker: whatever sits between the two is
        // either nothing (the bare form) or a type word — anything with a dash
        // in it is not a header.
        if let Some(open) = text[..end].rfind(PEM_HEAD) {
            let between = &text[open + PEM_HEAD.len()..end];
            if between.len() <= PEM_TYPE_MAX && !between.contains('-') {
                bump(&mut counts, "private-key");
            }
        }
        from = end + PEM_MARKER.len();
    }

    // Most-seen first, then by name: two runs over the same body render the
    // same note, which is what makes a note comparable across requests.
    counts.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.rule.cmp(b.rule)));
    counts
}

/// The note for a set of findings, or `None` when nothing fired.
///
/// One line per rule, and the rule name is all of it — see the module doc on
/// why no matched text and no value hash. The hash is the tempting one: it
/// would let the same secret be correlated across requests, but it is only
/// worth having if it cannot be reversed, and a hash of a low-entropy value is
/// reversible. That needs a keyed hash and a key to keep, which is a decision
/// this pass does not get to make on its own.
pub fn render(findings: &[Finding]) -> Option<String> {
    if findings.is_empty() {
        return None;
    }
    let mut lines: Vec<String> = findings
        .iter()
        .take(MAX_RULES)
        .map(|f| format!("dlp: {} ×{}", f.rule, f.count))
        .collect();
    if findings.len() > MAX_RULES {
        lines.push(format!(
            "dlp: … and {} more rule(s)",
            findings.len() - MAX_RULES
        ));
    }
    Some(lines.join("\n"))
}

fn bump(counts: &mut Vec<Finding>, rule: &'static str) {
    match counts.iter_mut().find(|f| f.rule == rule) {
        Some(f) => f.count += 1,
        None => counts.push(Finding { rule, count: 1 }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(text: &str) -> Vec<&'static str> {
        scan(text.as_bytes(), None)
            .into_iter()
            .map(|f| f.rule)
            .collect()
    }

    #[test]
    fn each_rule_fires_on_its_own_shape() {
        for (text, rule) in [
            ("sk-ant-api03-abcdefghijklmnopqrstuvwxyz", "anthropic-key"),
            ("sk-proj-abcdefghijklmnopqrstuvwxyz", "openai-key"),
            ("sk-abcdefghijklmnopqrstuvwxyz", "openai-key"),
            ("ghp_abcdefghijklmnopqrstuvwxyz", "github-token"),
            ("github_pat_abcdefghijklmnopqrstuvwxyz", "github-token"),
            ("glpat-abcdefghijklmnopqrstuvwxyz", "gitlab-token"),
            ("xoxb-abcdefghijklmnopqrstuvwxyz", "slack-token"),
            ("AKIAIOSFODNN7EXAMPLE", "aws-access-key"),
            ("npm_abcdefghijklmnopqrstuvwxyz", "npm-token"),
            ("pypi-abcdefghijklmnopqrstuvwxyz", "pypi-token"),
            ("dckr_pat_abcdefghijklmnopqrstuvwxyz", "docker-token"),
            ("AIzaSyabcdefghijklmnopqrstuvwxyz0123", "google-api-key"),
            (
                "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.abcdefghijklmnop",
                "jwt",
            ),
            ("-----BEGIN RSA PRIVATE KEY-----", "private-key"),
            ("-----BEGIN OPENSSH PRIVATE KEY-----", "private-key"),
        ] {
            assert_eq!(rules(text), vec![rule], "{text}");
        }
    }

    #[test]
    fn stripe_shapes_fire() {
        // Built at runtime, prefix + suffix as separate literals: push
        // secret-scanning flags the literal shape, and a test must never carry
        // one of the tokens it demonstrates how to catch.
        for text in ["sk_", "rk_"] {
            let full = String::from(text) + "live_abcdefghijklmnopqrstuvwxyz";
            assert_eq!(rules(full.as_str()), vec!["stripe-key"], "{full}");
        }
    }

    #[test]
    fn a_prefix_inside_a_word_is_not_a_key() {
        // The boundary rule: `task-sk-…` is an identifier, not a credential.
        assert!(rules("task-sk-abcdefghijklmnopqrstuvwxyz").is_empty());
        assert!(rules("my_AUTHORIZATION_token_value_here").is_empty());
    }

    #[test]
    fn our_own_placeholder_is_not_reported() {
        // `kw-ag-` is in every takeover's agent config. Reporting it would fire
        // on nearly every request, so it is a redaction rule and not a finding.
        assert!(rules("kw-ag-abcdefghijklmnop").is_empty());
        assert!(rules("Authorization: Bearer kw-ag-abcdefghijklmnop").is_empty());
    }

    #[test]
    fn a_bare_bearer_is_not_reported() {
        // An authorization scheme seen in prose, not a credential.
        assert!(rules("send it as Bearer abcdefghijklmnopqrstuvwxyz").is_empty());
    }

    #[test]
    fn a_short_tail_does_not_count() {
        // Enough of a prefix to look like one, not enough material to be a key.
        assert!(rules("sk-abcdef").is_empty());
        assert!(rules("AKIASHORT").is_empty());
    }

    #[test]
    fn a_marker_quoted_in_prose_is_not_a_key() {
        assert!(rules("the header is PRIVATE KEY----- in some formats").is_empty());
    }

    #[test]
    fn findings_are_counted_and_ordered() {
        let text = "ghp_abcdefghijklmnopqrstuvwxyz and sk-abcdefghijklmnopqrstuvwxyz \
                    and ghp_zyxwvutsrqponmlkjihgfedcba";
        assert_eq!(
            scan(text.as_bytes(), None),
            vec![
                Finding {
                    rule: "github-token",
                    count: 2
                },
                Finding {
                    rule: "openai-key",
                    count: 1
                },
            ]
        );
    }

    #[test]
    fn the_cap_bounds_what_is_scanned() {
        let text = "padpadpadpadpadpadpadpadpadpad sk-abcdefghijklmnopqrstuvwxyz";
        let key_at = text.find("sk-").expect("the key is in the fixture");
        // A cap that stops before the key sees nothing at all.
        assert!(scan(text.as_bytes(), Some(key_at)).is_empty());
        // One that reaches it sees it.
        assert_eq!(
            scan(text.as_bytes(), Some(text.len())),
            vec![Finding {
                rule: "openai-key",
                count: 1
            }]
        );
    }

    #[test]
    fn the_note_names_rules_and_counts_and_no_matched_text() {
        let text = "ghp_abcdefghijklmnopqrstuvwxyz";
        let note = render(&scan(text.as_bytes(), None)).expect("a finding");
        assert_eq!(note, "dlp: github-token ×1");
        // The reflexive rule: the value that was caught is not in the note.
        assert!(!note.contains("ghp_abcdefghijklmnopqrstuvwxyz"), "{note}");
    }

    #[test]
    fn nothing_found_is_no_note_at_all() {
        assert_eq!(render(&scan(b"just a request", None)), None);
    }

    #[test]
    fn the_rule_lines_are_capped_and_the_overflow_is_named() {
        let many: Vec<Finding> = (0..MAX_RULES + 2)
            .map(|i| Finding {
                rule: "r",
                count: i,
            })
            .collect();
        let note = render(&many).expect("findings");
        let lines: Vec<&str> = note.lines().collect();
        assert_eq!(lines.len(), MAX_RULES + 1);
        assert!(
            lines.last().unwrap().contains("and 2 more rule(s)"),
            "{note}"
        );
    }
}
