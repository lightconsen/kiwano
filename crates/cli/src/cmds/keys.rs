//! Placeholder-key management for a provider.

use super::render::render_keys;
use super::runtime;
use crate::cli::KeysCmd;
use crate::{CliError, Ctx};
use kiwano_core::vm;

// ── keys ────────────────────────────────────────────────────────────────────

pub fn keys(cmd: &KeysCmd, ctx: &mut Ctx) -> Result<(), CliError> {
    match cmd {
        KeysCmd::List { provider_id } => {
            let keys = {
                let store = ctx.store()?;
                vm::list_api_keys(store, provider_id)?
            };
            let text = render_keys(&keys, provider_id);
            ctx.out.emit(&keys, || text);
            Ok(())
        }
        KeysCmd::Add {
            provider_id,
            key,
            label,
        } => {
            let entry = {
                let store = ctx.store()?;
                vm::add_api_key(store, provider_id, key, label.as_deref())?
            };
            let text = format!("added key #{} ({})", entry.id, entry.masked);
            ctx.out.emit(&entry, || text);
            ctx.after_mutation();
            Ok(())
        }
        KeysCmd::Remove { key_id } => {
            let removed = {
                let store = ctx.store()?;
                vm::delete_api_key(store, *key_id)?
            };
            if !removed {
                return Err(runtime(format!("key not found: {key_id}")));
            }
            ctx.out.line(format!("removed key #{key_id}"));
            ctx.after_mutation();
            Ok(())
        }
    }
}
