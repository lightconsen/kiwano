//! DeepSeek Harness (dsh): one row in each profile's `cordis.patch.yml`, plus
//! the `.env` the row's credential reference reads.
//!
//! dsh boots from shipped rows and then applies a patch list — a YAML block
//! sequence of `{id, config}` entries, each replacing the whole config of the
//! row with that id. The endpoint is therefore a *row* to take over, not a
//! provider table to add to: the gateway's address and key go into the config
//! of `llm-deepseek`, the row the shipped DeepSeek adapter is bound to.
//!
//! This module edits the file as **lines**, never through a YAML parser, and
//! that is a requirement rather than a preference: a profile's patch list is a
//! live-reloaded user layer full of comments and `!!js` expressions, and dsh
//! replaces a row's config wholesale — so the row we do not own has to come out
//! byte-for-byte as it went in. The classifier below is the one magpie
//! (github.com/yetone/magpie, internal/agent/dsh.go) proved out on real
//! profile files.
//!
//! The model is deliberately not written anywhere: `agent-default-model`
//! carries the user's own choice, stored settings win over the patch list
//! anyway, and the row we rewrite is the DeepSeek one — so a takeover changes
//! where DeepSeek requests go and nothing else.

/// The shipped row whose config carries the endpoint and the credential name.
pub const DSH_ROW_ID: &str = "llm-deepseek";

/// The credential variable the row names, and the `.env` line it is read
/// from. The same name qwen's entry points at (`qwen.rs`), for the same reason:
/// one name for "the key Kiwano minted", wherever an agent reads it from.
pub const DSH_KEY_ENV: &str = "KIWANO_GATEWAY_KEY";

/// The trailing comment that marks an entry as ours. A row carrying magpie's
/// own marker (`# magpie`) is *not* ours and is left alone.
pub const DSH_MARK: &str = "# kiwano-gateway";

/// dsh's legacy single-file config, which this takeover does not support: the
/// profile patch layer replaced it in 0.1.5, and its rows are a different
/// schema (an inline `apiKey`, an `agent-loop` row).
pub const DSH_LEGACY_CONFIG: &str = "config.yaml";

/// One entry of the patch list, as its raw lines.
struct PatchItem {
    id: String,
    ours: bool,
    lines: Vec<String>,
}

/// A patch list split into the lines before its first entry and the entries
/// themselves.
struct PatchList {
    head: Vec<String>,
    items: Vec<PatchItem>,
}

/// The id of a `- id: x` line, and whether the line's trailing comment marks
/// the entry as ours. Only the two indentations dsh emits are recognized.
fn parse_id_line(line: &str) -> Option<(String, bool)> {
    let rest = if let Some(r) = line.strip_prefix("- ") {
        r
    } else if let Some(r) = line.strip_prefix("  ") {
        r
    } else {
        return None;
    };
    let rest = rest.strip_prefix("id:")?;
    let (value, comment) = match rest.split_once('#') {
        Some((v, c)) => (v, Some(c)),
        None => (rest, None),
    };
    let id = value
        .trim()
        .trim_matches(|c| c == '\'' || c == '"')
        .to_string();
    if id.is_empty() || id.contains(char::is_whitespace) {
        return None;
    }
    Some((id, comment.map(str::trim) == Some(DSH_MARK)))
}

/// Split a patch list the way dsh reads it: `- ` or `-` starts an entry, an
/// indented line belongs to the entry above it, and anything else at column
/// zero is a shape this takeover will not edit (an error, never a rewrite).
fn parse_patch_list(raw: &str) -> Result<PatchList, String> {
    let mut head: Vec<String> = Vec::new();
    let mut items: Vec<PatchItem> = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if line.starts_with("- ") || line == "-" {
            items.push(PatchItem {
                id: String::new(),
                ours: false,
                lines: vec![line.to_string()],
            });
        } else if items.is_empty() {
            // Before the first entry: comments and blank lines are the file's
            // own, and `[]` is the empty list dsh's template ships (dropped
            // here and re-synthesized if the list ends up empty again).
            if trimmed.is_empty() || trimmed.starts_with('#') || trimmed == "[]" {
                if trimmed != "[]" {
                    head.push(line.to_string());
                }
                continue;
            }
            return Err(format!(
                "the patch list is not a list of entries this app can edit: {trimmed:?}"
            ));
        } else if !trimmed.is_empty() && !trimmed.starts_with('#') && !line.starts_with(' ') {
            return Err(format!(
                "the patch list is not a list of entries this app can edit: {trimmed:?}"
            ));
        } else {
            items
                .last_mut()
                .expect("an entry is open")
                .lines
                .push(line.to_string());
        }
        // The id is read from whichever line carries it, first one wins.
        if let Some((id, ours)) = parse_id_line(line) {
            let item = items.last_mut().expect("an entry is open");
            if item.id.is_empty() {
                item.id = id;
                item.ours = ours;
            }
        }
    }
    Ok(PatchList { head, items })
}

fn render(list: &PatchList) -> String {
    let mut out = String::new();
    for line in &list.head {
        out.push_str(line);
        out.push('\n');
    }
    if list.items.is_empty() {
        // dsh wants a list even when it is empty.
        out.push_str("[]\n");
        return out;
    }
    for item in &list.items {
        for line in &item.lines {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// The entry we write: the DeepSeek row pointed at the gateway, its credential
/// named rather than inlined (the key itself lives in the `.env` beside it).
fn gateway_row_lines(base_url: &str) -> Vec<String> {
    vec![
        format!("- id: {DSH_ROW_ID} {DSH_MARK}"),
        "  config:".to_string(),
        format!("    apiKeyEnv: {DSH_KEY_ENV}"),
        format!("    baseURL: {base_url}"),
    ]
}

/// Point dsh's DeepSeek row at the gateway. The row is replaced where it
/// stands (dsh applies entries in order) or appended when the profile's patch
/// list has none; every other entry keeps its bytes.
pub fn upsert_dsh_patch_row(content: &str, base_url: &str) -> Result<String, String> {
    let mut list = parse_patch_list(content)?;
    let lines = gateway_row_lines(base_url);
    match list.items.iter().position(|i| i.id == DSH_ROW_ID) {
        Some(i) => {
            list.items[i] = PatchItem {
                id: DSH_ROW_ID.to_string(),
                ours: true,
                lines,
            }
        }
        None => list.items.push(PatchItem {
            id: DSH_ROW_ID.to_string(),
            ours: true,
            lines,
        }),
    }
    Ok(render(&list))
}

/// The `.env` line that carries the key: replaced where it stands, appended
/// otherwise. Every other line — including a user's own `DEEPSEEK_API_KEY` —
/// keeps its bytes.
pub fn upsert_dsh_env(content: &str, key: &str) -> Result<String, String> {
    let line = format!("{DSH_KEY_ENV}={key}");
    let mut out = String::new();
    let mut replaced = false;
    for existing in content.lines() {
        let is_ours = existing
            .trim_start()
            .strip_prefix(DSH_KEY_ENV)
            .is_some_and(|rest| rest.trim_start().starts_with('='));
        if is_ours && !replaced {
            out.push_str(&line);
            replaced = true;
        } else {
            out.push_str(existing);
        }
        out.push('\n');
    }
    if !replaced {
        out.push_str(&line);
        out.push('\n');
    }
    Ok(out)
}

/// Whether dsh's own settings pin an endpoint or key for the DeepSeek row.
/// Settings win over the patch list per request, so a takeover could be
/// "enabled" and still route nothing — the caller refuses instead of pretending.
///
/// Deliberately a line scan and not a parse: `settings.yaml` is dsh's own file,
/// we neither edit nor validate it, and all this needs is the presence of one
/// of the three keys under the row's section.
pub fn dsh_settings_conflict(settings: &str) -> Option<String> {
    let mut in_row = false;
    for line in settings.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if !line.starts_with(' ') && !line.starts_with('\t') {
            in_row = trimmed
                .strip_prefix(DSH_ROW_ID)
                .is_some_and(|rest| rest.trim_start().starts_with(':'));
            continue;
        }
        if !in_row {
            continue;
        }
        for key in ["baseURL:", "apiKey:", "apiKeyEnv:"] {
            if let Some(rest) = trimmed.strip_prefix(key) {
                if !rest.trim().is_empty() {
                    return Some(key.trim_end_matches(':').to_string());
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEMPLATE: &str = "# Your patch layer for this dsh profile.\n[]\n";

    const PROFILE: &str = "# web profile patches\n- id: my-own-row\n  config:\n    thing: !!js process.env.THING ?? undefined\n- id: llm-deepseek\n  name: '@deepseek-ai/dsh-llm-deepseek-api-key'\n";

    #[test]
    fn dsh_upserts_the_row_and_leaves_every_other_byte() {
        let out = upsert_dsh_patch_row(PROFILE, "http://127.0.0.1:8317/v1").unwrap();
        assert!(out.starts_with("# web profile patches\n"));
        // The user's own row survives byte-for-byte, `!!js` and all.
        assert!(out.contains("    thing: !!js process.env.THING ?? undefined\n"));
        // Ours replaces the shipped DeepSeek row in place.
        assert!(out.contains("- id: llm-deepseek # kiwano-gateway\n"));
        assert!(out.contains(&format!("    apiKeyEnv: {DSH_KEY_ENV}\n")));
        assert!(out.contains("    baseURL: http://127.0.0.1:8317/v1\n"));
        assert_eq!(out.matches("- id: llm-deepseek").count(), 1);
    }

    #[test]
    fn dsh_rows_are_appended_to_a_template_list() {
        let out = upsert_dsh_patch_row(TEMPLATE, "http://127.0.0.1:8317/v1").unwrap();
        assert_eq!(out, "# Your patch layer for this dsh profile.\n- id: llm-deepseek # kiwano-gateway\n  config:\n    apiKeyEnv: KIWANO_GATEWAY_KEY\n    baseURL: http://127.0.0.1:8317/v1\n");
    }

    #[test]
    fn dsh_rerun_replaces_our_row_rather_than_adding_one() {
        let once = upsert_dsh_patch_row(PROFILE, "http://127.0.0.1:8317/v1").unwrap();
        let twice = upsert_dsh_patch_row(&once, "http://127.0.0.1:8317/v1").unwrap();
        assert_eq!(once, twice);
    }

    #[test]
    fn dsh_refuses_a_file_that_is_not_a_block_list() {
        let err = upsert_dsh_patch_row("llm-deepseek:\n  x: 1\n", "http://127.0.0.1:8317/v1")
            .unwrap_err();
        assert!(err.contains("not a list of entries"), "{err}");
    }

    #[test]
    fn dsh_env_upserts_the_key_line_and_keeps_the_rest() {
        let original = "# dsh's own\nDEEPSEEK_API_KEY=sk-real\n";
        let out = upsert_dsh_env(original, "kw-ag-dsh-abcd").unwrap();
        assert_eq!(
            out,
            "# dsh's own\nDEEPSEEK_API_KEY=sk-real\nKIWANO_GATEWAY_KEY=kw-ag-dsh-abcd\n"
        );
        // A re-run updates in place instead of appending a second line.
        let again = upsert_dsh_env(&out, "kw-ag-dsh-efgh").unwrap();
        assert_eq!(again.matches(DSH_KEY_ENV).count(), 1);
        assert!(again.contains("KIWANO_GATEWAY_KEY=kw-ag-dsh-efgh\n"));
    }

    #[test]
    fn dsh_settings_conflict_sees_an_endpoint_or_key_under_the_row() {
        assert_eq!(
            dsh_settings_conflict("llm-deepseek:\n  baseURL: https://api.deepseek.com\n"),
            Some("baseURL".to_string())
        );
        assert_eq!(
            dsh_settings_conflict("ui:\n  theme: dark\nllm-deepseek:\n  apiKeyEnv: MY_KEY\n"),
            Some("apiKeyEnv".to_string())
        );
        // Unrelated settings, and a same-named key under another section, are
        // not a conflict.
        assert_eq!(dsh_settings_conflict("ui:\n  apiKey: x\n"), None);
        assert_eq!(
            dsh_settings_conflict("llm-deepseek:\n  thinking: enabled\n"),
            None
        );
    }
}
