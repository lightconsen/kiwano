//! Placeholder-key management for a provider.

use super::render::render_keys;
use super::runtime;
use crate::cli::KeysCmd;
use crate::{CliError, Ctx};

// ── keys ────────────────────────────────────────────────────────────────────

pub fn keys(cmd: &KeysCmd, ctx: &mut Ctx) -> Result<(), CliError> {
    match cmd {
        KeysCmd::List { provider_id, days } => {
            let keys = ctx.api.list_api_keys(provider_id)?;
            // The spend window defaults to 30 days, the same default the client
            // keys' own column uses: a bare figure in a table gets read as
            // all-time, so the window is a parameter and its name is in the
            // header.
            let days = days.unwrap_or(30).max(0);
            let spend = ctx.api.list_upstream_key_spend(provider_id, Some(days))?;
            let text = render_keys(&keys, &spend, provider_id, days);
            // An object rather than the bare array this used to print: the
            // command answers two questions now (what is in the pool, what each
            // entry spent), and the second needs its window named. A JSON
            // consumer that kept the old reading of `keys` finds it under
            // `keys`, and `days` says what the spend is over.
            #[derive(serde::Serialize)]
            struct Pool<'a> {
                provider_id: &'a str,
                days: i64,
                keys: &'a [kiwano_core::vm::ApiKeyVm],
                spend: &'a [kiwano_core::vm::UpstreamKeySpendVm],
            }
            ctx.out.emit(
                &Pool {
                    provider_id,
                    days,
                    keys: &keys,
                    spend: &spend,
                },
                || text,
            );
            Ok(())
        }
        KeysCmd::Add {
            provider_id,
            key,
            label,
        } => {
            let entry = ctx.api.add_api_key(provider_id, key, label.as_deref())?;
            let text = format!("added key #{} ({})", entry.id, entry.masked);
            ctx.out.emit(&entry, || text);
            Ok(())
        }
        KeysCmd::Remove { key_id } => {
            let removed = ctx.api.delete_api_key(*key_id)?;
            if !removed {
                return Err(runtime(format!("key not found: {key_id}")));
            }
            ctx.out.line(format!("removed key #{key_id}"));
            Ok(())
        }
    }
}
