//! kiwano-gateway binary entry (tech.md §4.1): data plane :8317 + admin plane
//! :8310, both loopback-bound. Runs as a Tauri sidecar; configuration comes
//! from environment variables so the GUI can pass explicit ports/paths.

use std::path::PathBuf;
use std::sync::Arc;

use kiwano_gateway::server::{admin_plane_router, data_plane_router, GatewayState};
use kiwano_gateway::store::Store;

const DEFAULT_DATA_PORT: u16 = 8317;
const DEFAULT_ADMIN_PORT: u16 = 8310;
const DEFAULT_DB_SUBDIR: &str = ".kiwano";
const DEFAULT_DB_FILE: &str = "kiwano.db";

fn env_port(name: &str, default: u16) -> u16 {
    match std::env::var(name) {
        Ok(v) if !v.is_empty() => match v.parse::<u16>() {
            Ok(p) => p,
            Err(_) => {
                tracing::warn!(env = name, value = %v, default, "invalid port; using default");
                default
            }
        },
        _ => default,
    }
}

fn default_db_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(DEFAULT_DB_SUBDIR)
        .join(DEFAULT_DB_FILE)
}

fn db_path_from_env() -> PathBuf {
    match std::env::var("KIWANO_DB_PATH") {
        Ok(v) if !v.is_empty() => PathBuf::from(v),
        _ => default_db_path(),
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let data_port = env_port("KIWANO_DATA_PORT", DEFAULT_DATA_PORT);
    let admin_port = env_port("KIWANO_ADMIN_PORT", DEFAULT_ADMIN_PORT);
    let db_path = db_path_from_env();

    if let Some(dir) = db_path.parent() {
        if let Err(e) = std::fs::create_dir_all(dir) {
            tracing::error!(path = %dir.display(), error = %e, "cannot create database directory");
            std::process::exit(1);
        }
    }

    let store = match Store::open(&db_path) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(path = %db_path.display(), error = %e, "cannot open database");
            std::process::exit(1);
        }
    };
    let state = match GatewayState::new(store) {
        Ok(s) => Arc::new(s),
        Err(e) => {
            tracing::error!(error = %e, "cannot initialize gateway state");
            std::process::exit(1);
        }
    };

    let data_addr = std::net::SocketAddr::from(([127, 0, 0, 1], data_port));
    let admin_addr = std::net::SocketAddr::from(([127, 0, 0, 1], admin_port));
    let data_listener = match tokio::net::TcpListener::bind(data_addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!(addr = %data_addr, error = %e, "cannot bind data plane (port conflict?)");
            std::process::exit(1);
        }
    };
    let admin_listener = match tokio::net::TcpListener::bind(admin_addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!(addr = %admin_addr, error = %e, "cannot bind admin plane (port conflict?)");
            std::process::exit(1);
        }
    };

    let agents = state.route_table().routes.len();
    tracing::info!(
        data = %data_addr,
        admin = %admin_addr,
        db = %db_path.display(),
        agents_routed = agents,
        version = state.version,
        "kiwano-gateway ready"
    );

    // Sidecar stdout handshake (tech.md §4.6): one parseable ready line.
    println!("ready data=127.0.0.1:{data_port} admin=127.0.0.1:{admin_port}");
    use std::io::Write as _;
    let _ = std::io::stdout().flush();

    let data_app = data_plane_router(state.clone());
    let admin_app = admin_plane_router(state.clone());

    if let Err(e) = tokio::try_join!(
        axum::serve(data_listener, data_app),
        axum::serve(admin_listener, admin_app),
    ) {
        tracing::error!(error = %e, "gateway terminated");
        std::process::exit(1);
    }
}
