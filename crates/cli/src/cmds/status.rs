//! Status and reload: the two commands a shell runs first.

use super::render::render_status;
use super::runtime;
use crate::{CliError, Ctx, EXIT_NEGATIVE, EXIT_OK};
use kiwano_core::sidecar;
use kiwano_core::vm;

// ── status / reload ─────────────────────────────────────────────────────────

pub fn status(ctx: &mut Ctx) -> Result<i32, CliError> {
    let admin = ctx.admin.describe();
    match sidecar::status_for(&ctx.admin, ctx.token.as_deref()) {
        Some(mut v) => {
            let footer = footer_stats(ctx);
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
        // Gateway down is an answer, not a failure: exit 1 is how a script tells
        // the two apart, and that much is unchanged.
        //
        // What it no longer does is read the shared database for a fallback
        // report (`migrate.local.md` §14.1, decision D3). A client that answers
        // from a database of its own is answering about a *different* daemon the
        // moment it is pointed at another machine — and a remote one could not
        // open that file at all. It is the same failure class as §10.29's
        // mistyped address: refused and named, rather than silently substituted.
        None => {
            ctx.out
                .line(format!("gateway: not running (admin {admin})"));
            Ok(EXIT_NEGATIVE)
        }
    }
}

/// Today's totals — what the app's status bar shows. Asked of the daemon, which
/// owns the usage rows and is the only side that can answer for *its* database.
///
/// `None` when there is no answer, which is the gateway-down case above.
fn footer_stats(ctx: &Ctx) -> Option<vm::FooterStatsVm> {
    ctx.api.footer_stats(env!("CARGO_PKG_VERSION")).ok()
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
