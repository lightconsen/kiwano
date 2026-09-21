//! Volcengine Signature V4.
//!
//! A Volcengine variant of AWS SigV4 with two fatal differences from the
//! standard algorithm (per volc-openapi-demos/signature/java/Sign.java):
//!
//! 1. canonical headers / SignedHeaders use the FIXED order
//!    `host;x-date;x-content-sha256;content-type` (NOT alphabetical);
//! 2. the algorithm string is `HMAC-SHA256` (no `AWS4` prefix), the
//!    credential scope ends in `request` (not `aws4_request`), and the key
//!    derivation is `kDate = HMAC(SK, date)` with no prefix either.
//!
//! The canonical query is still alphabetically ordered (as in standard SigV4);
//! service = `ark`, POST, empty body.

use crate::plan_quota::volcengine::{
    VOLCENGINE_API_VERSION, VOLCENGINE_CONTENT_TYPE, VOLCENGINE_OPENAPI_HOST, VOLCENGINE_SERVICE,
    VOLCENGINE_SIGNED_HEADERS,
};

fn volc_sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(data))
}

/// HMAC-SHA256 (RFC 2104), implemented directly on sha2 to avoid pulling in
/// the hmac crate for this single use.
fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    const BLOCK: usize = 64;
    let mut norm = [0u8; BLOCK];
    if key.len() > BLOCK {
        norm[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        norm[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    for b in &norm {
        inner.update([b ^ 0x36]);
    }
    inner.update(data);
    let inner = inner.finalize();
    let mut outer = Sha256::new();
    for b in &norm {
        outer.update([b ^ 0x5c]);
    }
    outer.update(inner);
    outer.finalize().into()
}

/// RFC3986-unreserved-escape for the canonical query string.
fn volc_uri_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Canonical query sorted by key (`Action` < `Region` < `Version`), shared
/// verbatim between the signature and the request URL.
pub(crate) fn volcengine_canonical_query(action: &str, region: &str) -> String {
    let mut pairs = [
        ("Action", action),
        ("Region", region),
        ("Version", VOLCENGINE_API_VERSION),
    ];
    pairs.sort_by(|a, b| a.0.cmp(b.0));
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", volc_uri_encode(k), volc_uri_encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// Build the Volcengine Signature V4 `Authorization` header. Returns
/// `(Authorization, X-Date, X-Content-Sha256)` — all three go into the
/// request headers. `now` is a parameter for deterministic tests.
pub(crate) fn volcengine_sign(
    access_key_id: &str,
    secret_access_key: &str,
    region: &str,
    canonical_query: &str,
    body: &[u8],
    now: chrono::DateTime<chrono::Utc>,
) -> (String, String, String) {
    let x_date = now.format("%Y%m%dT%H%M%SZ").to_string();
    let short_date = now.format("%Y%m%d").to_string();
    let x_content_sha256 = volc_sha256_hex(body);

    // Fixed-order canonical headers (Volcengine-specific, NOT sorted).
    let canonical_headers = format!(
        "host:{VOLCENGINE_OPENAPI_HOST}\nx-date:{x_date}\nx-content-sha256:{x_content_sha256}\ncontent-type:{VOLCENGINE_CONTENT_TYPE}\n"
    );
    let canonical_request = format!(
        "POST\n/\n{canonical_query}\n{canonical_headers}\n{VOLCENGINE_SIGNED_HEADERS}\n{x_content_sha256}"
    );

    let credential_scope = format!("{short_date}/{region}/{VOLCENGINE_SERVICE}/request");
    let string_to_sign = format!(
        "HMAC-SHA256\n{x_date}\n{credential_scope}\n{}",
        volc_sha256_hex(canonical_request.as_bytes())
    );

    // Key derivation: kDate = HMAC(SK, date) (no AWS4 prefix), suffix `request`.
    let k_date = hmac_sha256(secret_access_key.as_bytes(), short_date.as_bytes());
    let k_region = hmac_sha256(&k_date, region.as_bytes());
    let k_service = hmac_sha256(&k_region, VOLCENGINE_SERVICE.as_bytes());
    let k_signing = hmac_sha256(&k_service, b"request");
    let signature: String = hmac_sha256(&k_signing, string_to_sign.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();

    let authorization = format!(
        "HMAC-SHA256 Credential={access_key_id}/{credential_scope}, SignedHeaders={VOLCENGINE_SIGNED_HEADERS}, Signature={signature}"
    );
    (authorization, x_date, x_content_sha256)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan_quota::volcengine::volcengine_region;

    #[test]
    fn hmac_sha256_matches_rfc4231_vector() {
        // RFC 4231 test case 2: key "Jefe", data "what do ya want for nothing?"
        let mac = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
        let hex: String = mac.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(
            hex,
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn volcengine_region_and_query_and_sign_contract() {
        assert_eq!(
            volcengine_region("https://ark.cn-beijing.volces.com/api/coding"),
            "cn-beijing"
        );
        assert_eq!(
            volcengine_region("https://example.com/api/coding"),
            "cn-beijing"
        );
        assert_eq!(
            volcengine_canonical_query("GetAFPUsage", "cn-beijing"),
            "Action=GetAFPUsage&Region=cn-beijing&Version=2024-01-01"
        );

        let now = chrono::DateTime::parse_from_rfc3339("2024-06-21T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let region = "cn-beijing";
        let query = volcengine_canonical_query("GetAFPUsage", region);
        let (auth, x_date, x_content) =
            volcengine_sign("AKLTtest", "secretkey", region, &query, b"", now);
        // Empty-body SHA-256.
        assert_eq!(
            x_content,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(x_date, "20240621T000000Z");
        // No AWS4 prefix, scope suffix `request`, fixed SignedHeaders order.
        assert!(
            auth.starts_with("HMAC-SHA256 Credential=AKLTtest/20240621/cn-beijing/ark/request,")
        );
        assert!(auth.contains("SignedHeaders=host;x-date;x-content-sha256;content-type,"));
        let sig = auth.rsplit("Signature=").next().unwrap();
        assert_eq!(sig.len(), 64);
        assert!(sig.bytes().all(|b| b.is_ascii_hexdigit()));
        // Deterministic.
        let (auth2, _, _) = volcengine_sign("AKLTtest", "secretkey", region, &query, b"", now);
        assert_eq!(auth, auth2);
    }
}
