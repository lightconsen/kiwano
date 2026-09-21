//! The gateway daemon lifecycle.

use super::runtime;
use crate::cli::GatewayCmd;
use crate::{CliError, Ctx};
use kiwano_core::sidecar;

// ── gateway daemon ──────────────────────────────────────────────────────────

pub fn gateway(cmd: &GatewayCmd, ctx: &mut Ctx) -> Result<(), CliError> {
    match cmd {
        GatewayCmd::Start => {
            let status = sidecar::status_for(&ctx.admin, ctx.token.as_deref());
            match sidecar::startup_action(status.as_ref(), env!("CARGO_PKG_VERSION")) {
                sidecar::StartupAction::Adopt => {
                    ctx.out.line(format!(
                        "gateway already running (admin {})",
                        ctx.admin.describe()
                    ));
                }
                sidecar::StartupAction::Restart => {
                    let child = sidecar::restart(&ctx.admin).map_err(runtime)?;
                    ctx.out
                        .line(format!("replaced the running gateway (pid {})", child.id()));
                }
                sidecar::StartupAction::Spawn => {
                    let child = sidecar::spawn().map_err(runtime)?;
                    ctx.out
                        .line(format!("started gateway (pid {})", child.id()));
                }
            }
            Ok(())
        }
        GatewayCmd::Stop => {
            if !sidecar::request_shutdown(&ctx.admin) {
                return Err(runtime(format!(
                    "the gateway would not stop (admin {}); stop it by hand if it is still running",
                    ctx.admin.describe()
                )));
            }
            ctx.out.line("gateway stopped");
            Ok(())
        }
        GatewayCmd::Restart => {
            let child = sidecar::restart(&ctx.admin).map_err(runtime)?;
            ctx.out
                .line(format!("gateway restarted (pid {})", child.id()));
            Ok(())
        }
    }
}
