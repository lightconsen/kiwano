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

use kiwano_api::agents::CustomAgentVm;
use kiwano_api::keys::ApiKeyVm;
use kiwano_api::routes::{AgentLimitVm, AgentRouteVm};
use kiwanod::api::logs::RequestLogListVm;
use kiwanod::store::{RequestLogDetail, RequestLogEntry, RequestLogFilter};

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

    /// Add a rotation key — `vm::add_api_key`. Safe to retry: the far end keys
    /// on the value, so a second delivery answers with the row already there
    /// rather than a second one (`migrate.local.md` §6.1).
    pub fn add_api_key(
        &self,
        provider_id: &str,
        api_key: &str,
        label: Option<&str>,
    ) -> Result<ApiKeyVm, String> {
        #[derive(serde::Serialize)]
        struct Body<'a> {
            api_key: &'a str,
            label: Option<&'a str>,
        }
        sidecar::admin_post_json(
            &self.endpoint,
            self.token.as_deref(),
            &format!("/api/providers/{provider_id}/keys"),
            &Body { api_key, label },
        )
    }

    /// Remove a rotation key — `vm::delete_api_key`. `false` when there was no
    /// such row, which is what a retry finds and is not an error.
    pub fn delete_api_key(&self, id: i64) -> Result<bool, String> {
        let deleted: serde_json::Value = sidecar::admin_delete_json(
            &self.endpoint,
            self.token.as_deref(),
            &format!("/api/keys/{id}"),
        )?;
        Ok(deleted
            .get("deleted")
            .and_then(|d| d.as_bool())
            .unwrap_or(false))
    }

    // ── Agent routes and bindings ──

    /// Every agent's route, its ordered candidates and its ceilings —
    /// `vm::build_agent_routes`.
    pub fn list_agent_routes(&self) -> Result<Vec<AgentRouteVm>, String> {
        sidecar::admin_get_json(&self.endpoint, self.token.as_deref(), "/api/agent-routes")
    }

    /// Set an agent's strategy, and the JSON payload some of them carry —
    /// `vm::set_agent_strategy`.
    pub fn set_agent_strategy(
        &self,
        agent: &str,
        strategy: &str,
        config: Option<&str>,
    ) -> Result<(), String> {
        #[derive(serde::Serialize)]
        struct Body<'a> {
            strategy: &'a str,
            config: Option<&'a str>,
        }
        wrote(sidecar::admin_post_json(
            &self.endpoint,
            self.token.as_deref(),
            &format!("/api/agents/{agent}/strategy"),
            &Body { strategy, config },
        ))
    }

    /// Replace an agent's whole ceiling set — `vm::set_agent_limits`. A `PUT`:
    /// the array *is* the resulting set, and an empty one clears it.
    pub fn set_agent_limits(&self, agent: &str, limits: &[AgentLimitVm]) -> Result<(), String> {
        wrote(sidecar::admin_put_json(
            &self.endpoint,
            self.token.as_deref(),
            &format!("/api/agents/{agent}/limits"),
            &limits,
        ))
    }

    /// Rewrite the candidate order — `vm::reorder_agent_bindings`.
    pub fn reorder_agent_bindings(
        &self,
        agent: &str,
        provider_ids: &[String],
    ) -> Result<(), String> {
        #[derive(serde::Serialize)]
        struct Body<'a> {
            provider_ids: &'a [String],
        }
        wrote(sidecar::admin_put_json(
            &self.endpoint,
            self.token.as_deref(),
            &format!("/api/agents/{agent}/bindings"),
            &Body { provider_ids },
        ))
    }

    /// Patch one binding's weight and time window — `vm::update_agent_binding`.
    /// A `PATCH`: a field left out keeps its value, which is what the screen's
    /// half-filled forms rely on.
    pub fn update_agent_binding(
        &self,
        agent: &str,
        provider_id: &str,
        weight: Option<i64>,
        win_start: Option<&str>,
        win_end: Option<&str>,
    ) -> Result<(), String> {
        #[derive(serde::Serialize)]
        struct Body<'a> {
            weight: Option<i64>,
            win_start: Option<&'a str>,
            win_end: Option<&'a str>,
        }
        wrote(sidecar::admin_patch_json(
            &self.endpoint,
            self.token.as_deref(),
            &format!("/api/agents/{agent}/bindings/{provider_id}"),
            &Body {
                weight,
                win_start,
                win_end,
            },
        ))
    }

    /// Bind a provider to an agent — `vm::add_agent_binding`. A no-op when it
    /// is already bound, so a retry cannot reorder the queue (`§6.1`).
    pub fn add_agent_binding(&self, agent: &str, provider_id: &str) -> Result<(), String> {
        #[derive(serde::Serialize)]
        struct Body<'a> {
            provider_id: &'a str,
        }
        wrote(sidecar::admin_post_json(
            &self.endpoint,
            self.token.as_deref(),
            &format!("/api/agents/{agent}/bindings"),
            &Body { provider_id },
        ))
    }

    /// Unbind one candidate — `vm::remove_agent_binding`. Unlike the key pool,
    /// removing what is not bound is an error; the daemon decides that, and the
    /// message travels back unchanged.
    pub fn remove_agent_binding(&self, agent: &str, provider_id: &str) -> Result<(), String> {
        wrote(sidecar::admin_delete_json(
            &self.endpoint,
            self.token.as_deref(),
            &format!("/api/agents/{agent}/bindings/{provider_id}"),
        ))
    }

    /// Copy one agent's route onto another — `vm::apply_agent_route`.
    pub fn apply_agent_route(&self, target: &str, source: &str) -> Result<(), String> {
        #[derive(serde::Serialize)]
        struct Body<'a> {
            source: &'a str,
        }
        wrote(sidecar::admin_post_json(
            &self.endpoint,
            self.token.as_deref(),
            &format!("/api/agents/{target}/route/apply"),
            &Body { source },
        ))
    }
    // ── User-defined agents ──

    /// Define a user-defined agent — `vm::add_custom_agent`.
    ///
    /// The **id is the caller's** (`kiwano_api::ids::mint_agent_id`): nothing
    /// else in the request can identify the operation, since the agent's id
    /// carries a random suffix, and an id the daemon has already seen is
    /// answered with the agent that exists (`migrate.local.md` §6.1). Passing it
    /// in is what lets a caller that retries hold the same id across attempts.
    pub fn add_custom_agent(
        &self,
        id: &str,
        label: &str,
        note: Option<&str>,
        protocol: Option<&str>,
    ) -> Result<CustomAgentVm, String> {
        #[derive(serde::Serialize)]
        struct Body<'a> {
            id: &'a str,
            label: &'a str,
            note: Option<&'a str>,
            protocol: Option<&'a str>,
        }
        sidecar::admin_post_json(
            &self.endpoint,
            self.token.as_deref(),
            "/api/custom-agents",
            &Body {
                id,
                label,
                note,
                protocol,
            },
        )
    }

    /// Rename one, re-note it, re-protocol it — `vm::update_custom_agent`. A
    /// `PUT`: the body is the resulting state, and the id is in the path because
    /// it is the one field that does not change.
    pub fn update_custom_agent(
        &self,
        id: &str,
        label: &str,
        note: Option<&str>,
        protocol: Option<&str>,
    ) -> Result<CustomAgentVm, String> {
        #[derive(serde::Serialize)]
        struct Body<'a> {
            label: &'a str,
            note: Option<&'a str>,
            protocol: Option<&'a str>,
        }
        sidecar::admin_put_json(
            &self.endpoint,
            self.token.as_deref(),
            &format!("/api/custom-agents/{id}"),
            &Body {
                label,
                note,
                protocol,
            },
        )
    }

    /// Delete an agent with its route and its key — `vm::remove_custom_agent`.
    /// Its usage history stays.
    pub fn remove_custom_agent(&self, id: &str) -> Result<(), String> {
        wrote(sidecar::admin_delete_json(
            &self.endpoint,
            self.token.as_deref(),
            &format!("/api/custom-agents/{id}"),
        ))
    }
    // ── Provider writes ──

    /// Switch a provider on or off — `vm::set_provider_enabled`.
    pub fn set_provider_enabled(&self, id: &str, enabled: bool) -> Result<(), String> {
        #[derive(serde::Serialize)]
        struct Body {
            enabled: bool,
        }
        wrote(sidecar::admin_put_json(
            &self.endpoint,
            self.token.as_deref(),
            &format!("/api/providers/{id}/enabled"),
            &Body { enabled },
        ))
    }

    /// Delete a provider — `vm::delete_provider`. `false` when there was no such
    /// row, which is what a retry finds and is not an error.
    pub fn delete_provider(&self, id: &str) -> Result<bool, String> {
        let deleted: serde_json::Value = sidecar::admin_delete_json(
            &self.endpoint,
            self.token.as_deref(),
            &format!("/api/providers/{id}"),
        )?;
        Ok(deleted
            .get("deleted")
            .and_then(|d| d.as_bool())
            .unwrap_or(false))
    }
    // ── Request logs and the credential banner ──

    /// One page of the audit trail — `vm::list_request_logs`.
    pub fn list_request_logs(
        &self,
        page: i64,
        page_size: i64,
        filter: RequestLogFilter<'_>,
    ) -> Result<RequestLogListVm, String> {
        let mut query = format!("page={page}&page_size={page_size}");
        // Absent filters are *omitted*, not sent empty: the far end reads an
        // empty value as a filter that matches nothing, which is a different
        // request from "no filter".
        for (key, value) in [
            ("agent", filter.agent),
            ("provider_id", filter.provider_id),
            ("status", filter.status),
            ("from", filter.from),
            ("to", filter.to),
        ] {
            if let Some(value) = value {
                query.push_str(&format!("&{key}={}", encode_query(value)));
            }
        }
        sidecar::admin_get_json(
            &self.endpoint,
            self.token.as_deref(),
            &format!("/api/logs?{query}"),
        )
    }

    /// One entry in full — `vm::get_request_log`. `None` when there is no such
    /// row, which is what a cleared trail answers.
    pub fn get_request_log(&self, id: i64) -> Result<Option<RequestLogDetail>, String> {
        sidecar::admin_get_json(
            &self.endpoint,
            self.token.as_deref(),
            &format!("/api/logs/{id}"),
        )
    }

    /// Empty the audit trail — `vm::clear_request_logs`. Idempotent.
    pub fn clear_request_logs(&self) -> Result<(), String> {
        wrote(sidecar::admin_delete_json(
            &self.endpoint,
            self.token.as_deref(),
            "/api/logs",
        ))
    }

    /// The newest unacknowledged credential finding — `vm::check_credential_finding`.
    pub fn check_credential_finding(&self) -> Result<Option<RequestLogEntry>, String> {
        sidecar::admin_get_json(
            &self.endpoint,
            self.token.as_deref(),
            "/api/credential-finding",
        )
    }

    /// Acknowledge a finding — `vm::ack_credential_finding`.
    pub fn ack_credential_finding(&self, id: i64) -> Result<(), String> {
        #[derive(serde::Serialize)]
        struct Body {
            id: i64,
        }
        wrote(sidecar::admin_post_json(
            &self.endpoint,
            self.token.as_deref(),
            "/api/credential-finding/ack",
            &Body { id },
        ))
    }
}

/// A value as a query-string component: everything outside the unreserved set is
/// percent-encoded.
///
/// Not decoration — the time filters are timestamps.
/// `2026-10-01T00:00:00+08:00` sent raw arrives with its `+` read as a space, and
/// the far end then compares against a string no row has. Encoding the whole
/// value is the only version of this that cannot be wrong for a character nobody
/// thought of.
fn encode_query(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// A write whose answer carries nothing a caller needs: `admin_send` already
/// turned a refusal into the `String` beside it, and the `{"ok":true}` body is
/// the convention, not information.
fn wrote(result: Result<serde_json::Value, String>) -> Result<(), String> {
    result.map(|_| ())
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
            // Read until the client stops writing, not once: headers and body
            // can arrive as separate segments, and a single `read` is how the
            // body would go missing from an assertion about the body.
            stream
                .set_read_timeout(Some(std::time::Duration::from_millis(250)))
                .unwrap();
            let mut request = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                match stream.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => request.extend_from_slice(&chunk[..n]),
                    Err(_) => break,
                }
            }
            stream.write_all(response.as_bytes()).unwrap();
            String::from_utf8_lossy(&request).into_owned()
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

    /// The write half: the body is JSON, the content type says so, and a
    /// deletion uses the verb that names the row rather than a body.
    /// The verbs the route commands use, on the wire.
    ///
    /// Three of these are new to the client (`PUT`, `PATCH`, and a `POST` whose
    /// body carries an optional field), and a verb is not decoration: `PUT`
    /// claims the body is the resulting state, `PATCH` claims an absent field
    /// means "leave it alone". A transport that quietly sent POST for both would
    /// still work against this daemon and would be wrong the moment a second
    /// implementation read it. So they are asserted here, at the seam where a
    /// mistake would be invisible.
    /// The create carries the **id the caller minted** — the whole of §6.1's
    /// idempotency for a command whose id is random. A client that let the
    /// daemon mint it would have no way to identify a retry, so this asserts the
    /// field is on the wire rather than that the call compiles.
    #[test]
    fn creating_an_agent_sends_the_id_the_caller_minted() {
        let body = r#"{"id":"long-tasks-3f9a1c","label":"Long Tasks","note":null,"placeholder_key":"kw-ag-long-tasks-3f9a1c-ab12","protocol":null}"#;
        let response: &'static str = Box::leak(
            format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
                body.len()
            )
            .into_boxed_str(),
        );
        let (_dir, endpoint, handle) = stub(response);
        let api = DaemonApi::with_token(endpoint, Some("tok-1".into()));

        let created = api
            .add_custom_agent("long-tasks-3f9a1c", "Long Tasks", None, None)
            .unwrap();
        assert_eq!(created.id, "long-tasks-3f9a1c");

        let request = handle.join().unwrap();
        assert!(
            request.starts_with("POST /api/custom-agents HTTP/1.1"),
            "{request}"
        );
        assert!(
            request.contains(r#"{"id":"long-tasks-3f9a1c","label":"Long Tasks","#),
            "{request}"
        );

        // And a rename is a PUT whose body is the resulting state: the id is in
        // the path, because it is the one field that does not change.
        let (_d2, e2, h2) = stub(response);
        DaemonApi::with_token(e2, Some("tok-1".into()))
            .update_custom_agent(
                "long-tasks-3f9a1c",
                "Nightly",
                Some("moved"),
                Some("gemini"),
            )
            .unwrap();
        let request = h2.join().unwrap();
        assert!(
            request.starts_with("PUT /api/custom-agents/long-tasks-3f9a1c HTTP/1.1"),
            "{request}"
        );
        assert!(
            request.contains(r#"{"label":"Nightly","note":"moved","protocol":"gemini"}"#),
            "{request}"
        );
    }

    /// The list's query string, filters and all — and the one character that
    /// makes this test worth writing: a time filter with a `+` in its offset.
    ///
    /// `2026-10-01T00:00:00+08:00` sent raw arrives as `2026-10-01T00:00:00
    /// 08:00` (a space), and the far end then filters against a string no row
    /// has — a silently empty page rather than an error. An absent filter is
    /// omitted rather than sent empty, for the same class of reason: the daemon
    /// reads an empty value as "match nothing".
    #[test]
    fn the_log_query_string_omits_absent_filters_and_encodes_the_rest() {
        let body = r#"{"rows":[],"total":0}"#;
        let response: &'static str = Box::leak(
            format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
                body.len()
            )
            .into_boxed_str(),
        );
        let (_dir, endpoint, handle) = stub(response);
        let api = DaemonApi::with_token(endpoint, Some("tok-1".into()));

        let page = api
            .list_request_logs(
                2,
                50,
                RequestLogFilter {
                    agent: Some("claude"),
                    provider_id: None,
                    status: Some("error"),
                    from: Some("2026-10-01T00:00:00+08:00"),
                    to: None,
                },
            )
            .unwrap();
        assert_eq!(page.total, 0);

        let request = handle.join().unwrap();
        let line = request.lines().next().unwrap();
        assert!(
            line.starts_with("GET /api/logs?page=2&page_size=50&agent=claude&status=error&from="),
            "{line}"
        );
        assert!(
            line.contains("from=2026-10-01T00%3A00%3A00%2B08%3A00"),
            "the offset's `+` must be encoded: {line}"
        );
        assert!(
            !line.contains("provider_id") && !line.contains("to="),
            "an absent filter is omitted, not sent empty: {line}"
        );
    }

    #[test]
    fn the_route_writes_use_the_verbs_the_contract_names() {
        let ok: &'static str = "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 11\r\n\r\n{\"ok\":true}";

        // PUT: the ceiling set is the whole state.
        let (_d1, e1, h1) = stub(ok);
        DaemonApi::with_token(e1, Some("tok-1".into()))
            .set_agent_limits(
                "claude",
                &[kiwano_api::routes::AgentLimitVm {
                    period: "day".into(),
                    period_limit: 100.0,
                    limit_unit: None,
                }],
            )
            .unwrap();
        let request = h1.join().unwrap();
        assert!(
            request.starts_with("PUT /api/agents/claude/limits HTTP/1.1"),
            "{request}"
        );
        assert!(
            request.contains(r#"[{"period":"day","period_limit":100.0,"limit_unit":null}]"#),
            "{request}"
        );

        // PATCH: an omitted field means "leave it alone", so it must not be
        // spelled as a null the far end would write.
        let (_d2, e2, h2) = stub(ok);
        DaemonApi::with_token(e2, Some("tok-1".into()))
            .update_agent_binding("claude", "p-ant", Some(7), None, None)
            .unwrap();
        let request = h2.join().unwrap();
        assert!(
            request.starts_with("PATCH /api/agents/claude/bindings/p-ant HTTP/1.1"),
            "{request}"
        );
        assert!(request.contains(r#""weight":7"#), "{request}");

        // DELETE: the path names the binding, so there is no body to send.
        let (_d3, e3, h3) = stub(ok);
        DaemonApi::with_token(e3, Some("tok-1".into()))
            .remove_agent_binding("claude", "p-ant")
            .unwrap();
        let request = h3.join().unwrap();
        assert!(
            request.starts_with("DELETE /api/agents/claude/bindings/p-ant HTTP/1.1"),
            "{request}"
        );
        assert!(!request.contains("Content-Type"), "{request}");

        // And a refusal keeps the daemon's own sentence, not a status code
        // retyped here — the app shows that string.
        let refusal: &'static str = "HTTP/1.1 404 Not Found\r\ncontent-type: application/json\r\ncontent-length: 79\r\n\r\n{\"ok\":false,\"error\":\"provider ghost is not bound to claude\",\"kind\":\"not_found\"}";
        let (_d4, e4, _h4) = stub(refusal);
        let err = DaemonApi::with_token(e4, Some("tok-1".into()))
            .remove_agent_binding("claude", "ghost")
            .map(|_| ())
            .unwrap_err();
        assert_eq!(err, "provider ghost is not bound to claude");
    }

    #[test]
    fn writes_send_json_and_the_right_verb() {
        let body = r#"{"id":7,"masked":"…abcd","label":null,"enabled":true,"created_at":"2026-01-01T00:00:00Z"}"#;
        let response: &'static str = Box::leak(
            format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
                body.len()
            )
            .into_boxed_str(),
        );
        let (_dir, endpoint, handle) = stub(response);
        let api = DaemonApi::with_token(endpoint, Some("tok-1".into()));

        let added = api
            .add_api_key("p-ant", "sk-rotated", Some("backup"))
            .unwrap();
        assert_eq!(added.id, 7);
        let request = handle.join().unwrap();
        assert!(
            request.starts_with("POST /api/providers/p-ant/keys HTTP/1.1"),
            "{request}"
        );
        assert!(
            request.contains("Content-Type: application/json"),
            "{request}"
        );
        assert!(
            request.contains(r#"{"api_key":"sk-rotated","label":"backup"}"#),
            "{request}"
        );

        let deleted_body = r#"{"deleted":true}"#;
        let response: &'static str = Box::leak(
            format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{deleted_body}",
                deleted_body.len()
            )
            .into_boxed_str(),
        );
        let (_dir2, endpoint2, handle2) = stub(response);
        let ok = DaemonApi::with_token(endpoint2, Some("tok-1".into()))
            .delete_api_key(7)
            .unwrap();
        assert!(ok);
        let request2 = handle2.join().unwrap();
        assert!(
            request2.starts_with("DELETE /api/keys/7 HTTP/1.1"),
            "{request2}"
        );
        // No body on a deletion: the path is the whole request.
        assert!(!request2.contains("Content-Type"), "{request2}");
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
