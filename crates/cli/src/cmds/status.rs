//! Status and reload: the two commands a shell runs first.

use super::render::render_status;
use super::runtime;
use crate::{CliError, Ctx, EXIT_NEGATIVE, EXIT_OK};
use kiwano_core::sidecar;
use kiwano_core::vm;

// ── status / reload ─────────────────────────────────────────────────────────

pub fn status(ctx: &mut Ctx) -> Result<i32, CliError> {
    let admin = ctx.admin.describe();
    let footer = footer_stats(ctx);
    match sidecar::status_for(&ctx.admin, ctx.token.as_deref()) {
        Some(mut v) => {
            // Additive rather than reshaped: the rest of this object is the
            // admin plane's report passed through, and a consumer parsing it
            // should keep working.
            if let (Some(object), Some(footer)) = (v.as_object_mut(), footer.as_ref()) {
                if let Ok(value) = serde_json::to_value(footer) {
                    object.insert("footer".to_string(), value);
                }
            }
            let text = render_status(&v, &admin, footer.as_ref());
            ctx.out.emit(&v, || text);
            Ok(EXIT_OK)
        }
        // Gateway down is an answer, not a failure: the store is still readable
        // and on a server that is exactly what you want to see. Exit 1 is how a
        // script tells the two apart.
        None => {
            ctx.out
                .line(format!("gateway: not running (admin {admin})"));
            // Only report on a store that is already there. Opening one creates
            // it as a side effect, and `status` is the first thing anyone runs
            // on a machine that has neither — it should not be the command that
            // leaves a database behind.
            if ctx.db.is_file() {
                let metrics = ctx.store()?.metrics().map_err(runtime)?;
                ctx.out.line(format!(
                    "store: providers {} · bindings {} · placeholder_keys {} · usage_rows {}",
                    metrics.providers,
                    metrics.bindings,
                    metrics.placeholder_keys,
                    metrics.usage_rows
                ));
            }
            if let Some(footer) = &footer {
                ctx.out.line(render_footer(footer));
            }
            Ok(EXIT_NEGATIVE)
        }
    }
}

/// Today's totals — what the app's status bar shows.
///
/// Returns `None` rather than forcing an answer when there is no database yet:
/// opening the store creates the file as a side effect, and `status` is the
/// first thing anyone runs on a machine that has neither, so it must stay a
/// read-only question.
fn footer_stats(ctx: &Ctx) -> Option<vm::FooterStatsVm> {
    if !ctx.db.is_file() {
        return None;
    }
    let (store, aux) = (ctx.store().ok()?, ctx.aux().ok()?);
    vm::build_footer_stats(store, aux, env!("CARGO_PKG_VERSION")).ok()
}

pub(crate) fn render_footer(footer: &vm::FooterStatsVm) -> String {
    format!(
        "today: {} requests · {} tokens · hub {} · v{}",
        footer.today_requests,
        vm::fmt_tokens(footer.today_tokens),
        if footer.hub_synced {
            "synced"
        } else {
            "not synced"
        },
        footer.version
    )
}

pub fn reload(ctx: &mut Ctx) -> Result<i32, CliError> {
    match sidecar::reload(&ctx.admin, ctx.token.as_deref()) {
        Some(v) if v["ok"].as_bool() == Some(true) => {
            ctx.out.line(format!(
                "reloaded: {} agents routed",
                v["agents_routed"].as_u64().unwrap_or(0)
            ));
            Ok(EXIT_OK)
        }
        Some(v) => Err(runtime(format!(
            "gateway refused the reload: {}",
            v["error"].as_str().unwrap_or("no reason given")
        ))),
        None => Err(runtime(format!(
            "gateway not reachable on {}",
            ctx.admin.describe()
        ))),
    }
}
