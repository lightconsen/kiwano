//! The client of the daemon's resource API — one function per command.
//!
//! `migrate.local.md` §7 batch 1 moves the *state* commands here: the app and
//! the CLI stop opening the store and ask the process that owns it. The shape
//! of each function is deliberately the same as the `vm::` function it
//! replaces (same arguments, same return type, same error string where there is
//! one), so a caller changes one line and no call site has to be reshaped
//! around the transport.
//!
//! Two things every one of these needs are in `sidecar`: the authorized GET
//! ([`crate::sidecar::admin_get_json`]) and the endpoint the app and the CLI
//! already resolve the same way.

use kiwano_api::keys::ApiKeyVm;

use crate::sidecar::{self, AdminEndpoint};

/// A client of one daemon: where it is, and what it authenticates with.
///
/// Both halves in one value because they are answered together — and because
/// the second is the open question of `migrate.local.md` §13.5. Today a client
/// reads the token from the database it shares with its daemon
/// ([`DaemonApi::connect`]); a remote one will have to be told, which is
/// [`DaemonApi::with_token`]. Every resource call takes a `DaemonApi`, so that
/// question has one place to be answered in rather than thirty.
pub struct DaemonApi {
    endpoint: AdminEndpoint,
    token: Option<String>,
}

impl DaemonApi {
    /// The client for a daemon on this machine: same database, therefore the
    /// same token.
    pub fn connect() -> Self {
        Self::with_token(sidecar::admin_endpoint(), sidecar::admin_token())
    }

    pub fn with_token(endpoint: AdminEndpoint, token: Option<String>) -> Self {
        Self { endpoint, token }
    }

    /// A provider's key pool, masked — `vm::list_api_keys`, served by the daemon.
    pub fn list_api_keys(&self, provider_id: &str) -> Result<Vec<ApiKeyVm>, String> {
        sidecar::admin_get_json(
            &self.endpoint,
            self.token.as_deref(),
            &format!("/api/providers/{provider_id}/keys"),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    /// A daemon's side of the socket, canned: whatever the test wants answered,
    /// plus the request it was asked with. The client's own half — the framing,
    /// the token header, the status check, the error envelope — is what these
    /// tests are for, and none of it needs a running gateway.
    fn stub(
        response: &'static str,
    ) -> (
        tempfile::TempDir,
        AdminEndpoint,
        std::thread::JoinHandle<String>,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("admin.sock");
        let listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        let endpoint = AdminEndpoint::beside_db(&dir.path().join("k.db"));
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 4096];
            let n = stream.read(&mut request).unwrap();
            stream.write_all(response.as_bytes()).unwrap();
            String::from_utf8_lossy(&request[..n]).into_owned()
        });
        (dir, endpoint, handle)
    }

    #[test]
    fn a_key_pool_comes_back_parsed() {
        let body = r#"[{"id":3,"masked":"sk-liv…mnop","label":"backup","enabled":true,"created_at":"2026-01-01T00:00:00Z"}]"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
            body.len()
        );
        let response: &'static str = Box::leak(response.into_boxed_str());
        let (_dir, endpoint, handle) = stub(response);

        let keys = DaemonApi::with_token(endpoint, Some("tok-123".into()))
            .list_api_keys("p-ant")
            .unwrap();

        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].id, 3);
        assert_eq!(keys[0].masked, "sk-liv…mnop");
        assert_eq!(keys[0].label.as_deref(), Some("backup"));
        // The request is part of the contract too: the path names the resource
        // and its owner, and the token travels in its own header.
        let request = handle.join().unwrap();
        assert!(
            request.starts_with("GET /api/providers/p-ant/keys HTTP/1.1"),
            "{request}"
        );
        assert!(
            request.contains("x-kiwano-admin-token: tok-123"),
            "{request}"
        );
    }

    /// A refusal arrives as the daemon's own sentence, not as a status code
    /// retyped here: the message is what a user can act on.
    #[test]
    fn a_refusal_keeps_the_daemons_own_message() {
        let body = r#"{"ok":false,"error":"no such provider: p-missing"}"#;
        let response = format!(
            "HTTP/1.1 500 Internal Server Error\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
            body.len()
        );
        let response: &'static str = Box::leak(response.into_boxed_str());
        let (_dir, endpoint, _handle) = stub(response);

        let err = DaemonApi::with_token(endpoint, None)
            .list_api_keys("p-missing")
            .unwrap_err();
        assert_eq!(err, "no such provider: p-missing");
    }
}
