//! Placeholder-key management for a provider.

use super::render::render_keys;
use super::runtime;
use crate::cli::KeysCmd;
use crate::{CliError, Ctx};

// ── keys ────────────────────────────────────────────────────────────────────

pub fn keys(cmd: &KeysCmd, ctx: &mut Ctx) -> Result<(), CliError> {
    match cmd {
        KeysCmd::List { provider_id } => {
            let keys = ctx.api.list_api_keys(provider_id)?;
            let text = render_keys(&keys, provider_id);
            ctx.out.emit(&keys, || text);
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
