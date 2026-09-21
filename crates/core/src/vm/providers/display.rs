//! How a provider's endpoint reads: the primary one with its scheme stripped
//! and `api_path` folded in, the protocol subtitle under it, and the additional
//! per-protocol endpoints in the same shape as the primary.

use super::types::ProviderEndpointVm;
use kiwanod::store::Provider;

fn display_base(base_url: &str, api_path: &Option<String>) -> String {
    let stripped = base_url
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    match api_path {
        Some(path) if !path.is_empty() => format!("{stripped}{path}"),
        _ => stripped.to_string(),
    }
}

pub(crate) fn display_endpoint(p: &Provider) -> String {
    display_base(&p.base_url, &p.api_path)
}

fn protocol_label(p: kiwanod::store::Protocol) -> &'static str {
    match p {
        kiwanod::store::Protocol::OpenAI => "OpenAI-compatible",
        kiwanod::store::Protocol::Anthropic => "Anthropic",
        kiwanod::store::Protocol::Gemini => "Gemini",
    }
}

pub(crate) fn endpoint_note(p: &Provider) -> String {
    let mut note = protocol_label(p.protocol).to_string();
    // Additional endpoints surface in the same subtitle: "OpenAI-compatible · +Anthropic".
    for e in &p.endpoints {
        let tag = match e.protocol {
            kiwanod::store::Protocol::OpenAI => "OpenAI",
            kiwanod::store::Protocol::Anthropic => "Anthropic",
            kiwanod::store::Protocol::Gemini => "Gemini",
        };
        note.push_str(" · +");
        note.push_str(tag);
    }
    note
}

pub(crate) fn vm_endpoints(p: &Provider) -> Vec<ProviderEndpointVm> {
    p.endpoints
        .iter()
        .map(|e| ProviderEndpointVm {
            protocol: e.protocol.as_str().to_string(),
            endpoint: display_base(&e.base_url, &e.api_path),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    // Both cases read the endpoint back off a `ProviderVm`, so they name this
    // module through `build_provider_vms` rather than calling `display_*`.
    use crate::vm::provider_edit::{
        add_provider, update_provider, BillingConfigInput, NewEndpointInput, NewProviderInput,
    };
    use crate::vm::providers::build_provider_vms;
    use crate::vm::test_support::{catalog_input, linkless_aux, live_home, no_vars, store};
    use crate::vm::Aux;
    use kiwanod::store::Store;

    fn stored_base(s: &Store, id: &str) -> String {
        s.get_provider(id).unwrap().expect("provider row").base_url
    }

    /// A stored endpoint is an absolute URL, whatever the form it was typed in.
    ///
    /// The dialog is handed `display_endpoint` — scheme stripped, `api_path`
    /// folded in — and hands it back on save, and nothing downstream adds a
    /// scheme: the gateway concatenates and `reqwest` refuses a relative URL. So
    /// opening a provider and saving it again used to leave a row that reads like
    /// a working provider and fails every request.
    #[test]
    fn a_saved_endpoint_keeps_its_scheme() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();

        // Typed as the dialog shows it. `absolute_endpoint` is the only thing
        // between that and the row, and this is the case that was broken.
        let mut bare = catalog_input("DeepSeek", "api.deepseek.com");
        bare.endpoints = vec![NewEndpointInput {
            protocol: "anthropic".into(),
            endpoint: "api.deepseek.com/anthropic".into(),
        }];
        let vm = add_provider(&s, &aux, &bare).unwrap();
        assert_eq!(stored_base(&s, &vm.id), "https://api.deepseek.com");
        let extra = s.get_provider(&vm.id).unwrap().unwrap().endpoints;
        assert_eq!(extra[0].base_url, "https://api.deepseek.com/anthropic");

        // …and the edit the dialog sends back is the same stripped form, which
        // is what used to strip the scheme off an already-correct row.
        let vm = update_provider(
            &s,
            &aux,
            std::path::Path::new("/tmp"),
            &vm.id,
            &bare,
            &no_vars(),
        )
        .unwrap();
        assert_eq!(stored_base(&s, &vm.id), "https://api.deepseek.com");

        // A local server is plain HTTP: guessing https there fails at the
        // handshake, before anything can say why.
        let local = catalog_input("Ollama", "localhost:11434");
        let vm = add_provider(&s, &aux, &local).unwrap();
        assert_eq!(stored_base(&s, &vm.id), "http://localhost:11434");
        let loopback = catalog_input("Local", "127.0.0.1:1234");
        let vm = add_provider(&s, &aux, &loopback).unwrap();
        assert_eq!(stored_base(&s, &vm.id), "http://127.0.0.1:1234");

        // An absolute URL is left exactly as it is, http and https alike.
        for (typed, stored) in [
            ("https://api.moonshot.cn", "https://api.moonshot.cn"),
            ("http://relay.internal", "http://relay.internal"),
        ] {
            let vm = add_provider(&s, &aux, &catalog_input("Typed", typed)).unwrap();
            assert_eq!(stored_base(&s, &vm.id), stored, "{typed}");
        }
    }

    #[test]
    fn provider_vm_carries_endpoints_and_note_suffix() {
        let s = store();
        let input = NewProviderInput {
            catalog_id: None,
            // No declared prices: this fixture is priced by the Hub's table.
            prices: None,
            name: "Qianfan".into(),
            api_key: "sk-x".into(),
            endpoint: "https://qianfan.baidubce.com/v2/tokenplan/personal".into(),
            protocol: "openai".into(),
            model_default: "qianfan-code-latest".into(),
            billing: "payg".into(),
            billing_config: BillingConfigInput {
                limit_value: None,
                limit_unit: None,
                reset_period: None,
                plan_limits: None,
            },
            agents: Some(vec![]),
            endpoints: vec![NewEndpointInput {
                protocol: "anthropic".into(),
                endpoint: "https://qianfan.baidubce.com/anthropic/coding".into(),
            }],
            advanced: None,
            plan_query: None,
        };
        let vm = add_provider(&s, &linkless_aux(), &input).unwrap();
        assert_eq!(vm.endpoints.len(), 1);
        assert_eq!(vm.endpoints[0].protocol, "anthropic");
        // display_base strips the scheme (same as the primary endpoint field)
        assert_eq!(
            vm.endpoints[0].endpoint,
            "qianfan.baidubce.com/anthropic/coding"
        );
        assert_eq!(vm.endpoint_note, "OpenAI-compatible · +Anthropic");
        // endpoints survive a fresh VM build from the store
        let aux = Aux::open_in_memory().unwrap();
        let vms = build_provider_vms(&s, &aux, live_home(&[]).path(), &no_vars()).unwrap();
        let loaded = vms.iter().find(|v| v.id == vm.id).unwrap();
        assert_eq!(loaded.endpoints.len(), 1);
        assert_eq!(loaded.endpoint_note, "OpenAI-compatible · +Anthropic");
    }
}
