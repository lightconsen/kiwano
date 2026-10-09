//! The contract's first fixtures — one command, frozen.
//!
//! `migrate.local.md` §10 lists four kinds of test the API migration needs.
//! This file is the skeleton of that suite, built on the smallest pure-state
//! command (`add_api_key`) so the shape is settled before 29 more are moved.
//!
//! # What has teeth here, and what cannot yet
//!
//! Only one of the four can be real while the API does not exist, and saying
//! which is which is the point of writing this now rather than after:
//!
//! | §10 | kind | today |
//! |---|---|---|
//! | (1) | compile-time: the app can no longer reach the store | **impossible** — it needs all 30 moved, then the `kiwanod` dependency can go |
//! | (2) | transport parity: the same request down both paths | **impossible** — there is no second path yet |
//! | (3) | N-1 compatibility: a previous version's request still answered | **this file** — the shape frozen here is what a later break goes red against |
//! | (4) | migration convergence: fresh install vs upgraded install | **half** — the fixture factory is reusable, the assertion needs the migration |
//!
//! Nothing here is a placeholder test that cannot fail: (1) and (2) are left
//! unwritten rather than faked, and the two tests below fail today if the
//! shapes they pin move — which is the whole job of a contract test.
//!
//! # The finding this file produced
//!
//! **The wire types have no home both sides can see.** `ApiKeyVm` lives in
//! `crates/core`, and `crates/gateway` does not depend on core (it depends on
//! `kiwano-adapters` only), so the daemon cannot name the type it is supposed
//! to serve. Three ways out, recorded in the plan: move the wire types into
//! `kiwano-adapters` (the existing leaf both sides already depend on), add a
//! `kiwano-api` crate (semantically right, one more member), or let the daemon
//! define its own DTOs and map them in core (duplication, and the mapping is
//! where drift lives). Also: `ApiKeyVm` derives `Serialize` only — a *response*
//! type that cannot be parsed back, which fixtures and N-1 tests both need.
//!
//! [`reconcile_takeovers`]: kiwano_core::vm::takeover::reconcile_takeovers

use kiwano_core::vm;
use kiwano_core::vm::provider_edit::NewProviderInput;
use kiwano_core::vm::Aux;
use kiwanod::store::Store;

fn fixture(name: &str) -> serde_json::Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/api")
        .join(name);
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("{} is not JSON: {e}", path.display()))
}

/// A store with one provider, in a temp directory.
///
/// The provider is created by **replaying a request fixture** through the same
/// entry point the add dialog uses. Two reasons, both about what a contract
/// test should be pinned to:
///
/// - a hand-built `Provider` (or `NewProviderInput`) breaks every time those
///   structs gain a field, and neither struct is the contract — the *request*
///   is, and it now lives in JSON where the wire will carry it;
/// - `NewProviderInput` derives `Deserialize` **only**, which is another
///   finding: as a request type it can be read from JSON but not written to it,
///   and a client that has to *send* one will need `Serialize` too.
fn store_with_provider() -> (tempfile::TempDir, Store, Aux, String) {
    let tmp = tempfile::tempdir().unwrap();
    let db = tmp.path().join("kiwano.db");
    let store = Store::open(&db).unwrap();
    let aux = Aux::open(&db).unwrap();

    let request: NewProviderInput = serde_json::from_value(fixture("add_provider.request.json"))
        .expect("the frozen request fixture must still be a valid request");
    let provider = vm::add_provider(&store, &request).unwrap();
    (tmp, store, aux, provider.id)
}

/// §10 (3): the shape a client receives is frozen, field by field.
///
/// The values cannot be frozen — `id` is an autoincrement and `created_at` is
/// now — so what is pinned is the **shape**: the field set and each field's
/// JSON type. That is what a later rename, drop or retype goes red against,
/// which is exactly the class of change a contract has to notice.
#[test]
fn the_add_api_key_response_shape_is_frozen() {
    let (_tmp, store, _aux, provider) = store_with_provider();
    let vm = vm::add_api_key(&store, &provider, "sk-rotated-1", Some("backup")).unwrap();
    let got = serde_json::to_value(&vm).unwrap();
    let want = fixture("add_api_key.response.json");

    let fields = |v: &serde_json::Value| {
        let mut keys: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        keys
    };
    assert_eq!(
        fields(&got),
        fields(&want),
        "the response's field set changed — that is a contract break, not a refactor"
    );
    for (key, want_value) in want.as_object().unwrap() {
        let got_value = &got[key];
        assert_eq!(
            std::mem::discriminant(got_value),
            std::mem::discriminant(want_value),
            "`{key}` changed JSON type: {got_value:?} vs the frozen {want_value:?}"
        );
    }
}

/// §10 (3), second half: the failure vocabulary is part of the contract.
///
/// These strings are what the client will show, and `migrate.local.md` §6.1
/// plans to turn them into a stable `kind` vocabulary rather than prose. Pinned
/// here so a reword is a decision rather than an accident — the same reason the
/// data plane's `kind` strings are asserted where they are produced.
#[test]
fn the_add_api_key_failure_vocabulary_is_frozen() {
    let (_tmp, store, _aux, provider) = store_with_provider();
    let want = fixture("add_api_key.errors.json");

    // `.map(|_| ())` because `ApiKeyVm` is not `Debug` — a `Result<T, E>` cannot
    // be unwrapped on the error side without it, and that is itself a finding:
    // the response types were built for one direction only.
    let empty = vm::add_api_key(&store, &provider, "   ", None)
        .map(|_| ())
        .unwrap_err();
    assert_eq!(empty, want["empty_key"].as_str().unwrap());

    let unknown = vm::add_api_key(&store, "no-such-provider", "sk-x", None)
        .map(|_| ())
        .unwrap_err();
    assert_eq!(unknown, want["unknown_provider"].as_str().unwrap());
}

/// §6.1: a replay mints one row, not two.
///
/// This was written `#[ignore]`d, because the behaviour it asserts did not
/// exist yet and a red test on `main` is not a plan: run with `-- --ignored` it
/// failed against the old store (`left: 2, right: 1`). The keys module landed
/// the fix — the identity of a rotation key is its value — so it runs now, and
/// it is the acceptance gate for that change rather than a description of it.
#[test]
fn a_replayed_add_api_key_does_not_mint_a_second_row() {
    let (_tmp, store, _aux, provider) = store_with_provider();
    vm::add_api_key(&store, &provider, "sk-rotated-1", Some("backup")).unwrap();
    vm::add_api_key(&store, &provider, "sk-rotated-1", Some("backup")).unwrap();
    assert_eq!(
        vm::list_api_keys(&store, &provider).unwrap().len(),
        1,
        "the same key added twice is one entry"
    );
}
