//! Scoped synthetic CA trust exists only in unit-test builds and this thread.
use crate::{
    credentials::Secret,
    mcp::{Action, ConnectedRequest, Report},
    mcp_http::{self, CredentialUpdate, HttpConfig},
    mcp_oauth,
    mcp_oauth_callback::CallbackReceiver,
    mcp_oauth_login::Transaction,
    mcp_oauth_registration, mcp_oauth_token,
};
use reqwest::{Certificate, Url};
use serde_json::json;
use std::{
    cell::RefCell,
    sync::{Arc, atomic::AtomicBool, mpsc},
    thread,
    time::Duration,
};

thread_local! { static ROOT: RefCell<Option<Certificate>> = const { RefCell::new(None) }; }
pub(crate) fn trust_async(builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
    ROOT.with(|root| match root.borrow().clone() {
        Some(cert) => builder.add_root_certificate(cert),
        None => builder,
    })
}
pub(crate) fn trust_blocking(
    builder: reqwest::blocking::ClientBuilder,
) -> reqwest::blocking::ClientBuilder {
    ROOT.with(|root| match root.borrow().clone() {
        Some(cert) => builder.add_root_certificate(cert),
        None => builder,
    })
}
struct Trust;
impl Trust {
    fn scoped(cert: Certificate) -> Self {
        ROOT.with(|root| {
            assert!(root.borrow().is_none());
            *root.borrow_mut() = Some(cert);
        });
        Self
    }
}
impl Drop for Trust {
    fn drop(&mut self) {
        ROOT.with(|root| *root.borrow_mut() = None);
    }
}
fn client() -> reqwest::blocking::Client {
    trust_blocking(reqwest::blocking::Client::builder())
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap()
}
fn ask(sender: &mpsc::Sender<ConnectedRequest>, action: Action, confirmed: bool) -> Report {
    let (response, receiver) = mpsc::channel();
    sender
        .send(ConnectedRequest {
            action,
            manual_confirmed: confirmed,
            response,
        })
        .unwrap();
    receiver
        .recv_timeout(Duration::from_secs(20))
        .unwrap()
        .unwrap()
}
#[test]
#[ignore = "requires disposable TLS OAuth proxy plus official SDK auth lifecycle fixture"]
fn https_discovery_registration_login_refresh_and_revocation() {
    let endpoint = std::env::var("ZIDEVTOOLS_MCP_TLS_ENDPOINT").unwrap();
    let url = Url::parse(&endpoint).unwrap();
    assert_eq!(url.scheme(), "https");
    assert_eq!(url.host_str(), Some("127.0.0.1"));
    let certificate = Certificate::from_pem(
        &std::fs::read(std::env::var("ZIDEVTOOLS_MCP_TLS_CERTIFICATE").unwrap()).unwrap(),
    )
    .unwrap();
    // Public production entry must reject the untrusted CA before scoped test trust.
    assert!(mcp_oauth::discover_address(&endpoint).is_err());
    let _trust = Trust::scoped(certificate.clone());
    // Hostname checks remain enabled even after adding the test CA.
    let mut wrong_host = url.clone();
    wrong_host.set_host(Some("localhost")).unwrap();
    assert!(client().get(wrong_host).send().is_err());
    let discovery = mcp_oauth::discover_address(&endpoint).unwrap();
    assert!(discovery.advertised);
    let resource = mcp_oauth::fetch_resource(&endpoint, &discovery.metadata_url).unwrap();
    assert_eq!(resource.resource, endpoint);
    let auth = mcp_oauth::fetch_authorization(&resource.authorization_servers[0]).unwrap();
    let callback = CallbackReceiver::bind().unwrap();
    let cancel = AtomicBool::new(false);
    let registered = mcp_oauth_registration::register(
        &auth,
        &endpoint,
        callback.redirect_uri(),
        &["fixture:read".into()],
        &cancel,
    )
    .unwrap();
    let transaction = Transaction::prepare(
        &auth,
        &endpoint,
        &registered.client_id,
        callback.redirect_uri(),
        &registered.scopes,
    )
    .unwrap();
    let authorization = transaction.authorization_url().clone();
    let browser_certificate = certificate.clone();
    let browser = thread::spawn(move || {
        let _trust = Trust::scoped(browser_certificate);
        let client = client();
        let response = client.get(authorization).send().unwrap();
        assert_eq!(response.status().as_u16(), 302);
        let callback = Url::parse(response.headers()["location"].to_str().unwrap()).unwrap();
        assert_eq!(callback.scheme(), "http");
        assert_eq!(callback.host_str(), Some("127.0.0.1"));
        assert_eq!(callback.path(), "/oauth/callback/zi-devtools");
        assert_eq!(client.get(callback).send().unwrap().status().as_u16(), 200);
    });
    let grant = callback.receive(transaction, &cancel).unwrap();
    browser.join().unwrap();
    let token = mcp_oauth_token::exchange(grant, &cancel).unwrap();
    assert_eq!(token.bindings().0, endpoint);
    let credential = Secret::new(token.access_for(&endpoint).unwrap().expose().into()).unwrap();
    let (sender, requests) = mpsc::channel();
    let (updates, changes) = mpsc::channel();
    let worker_endpoint = endpoint.clone();
    let worker_certificate = certificate.clone();
    let worker = thread::spawn(move || {
        let _trust = Trust::scoped(worker_certificate);
        mcp_http::serve_http_with_updates(
            HttpConfig {
                endpoint: worker_endpoint,
            },
            Some(credential),
            Arc::new(AtomicBool::new(false)),
            requests,
            changes,
        )
        .unwrap();
    });
    let report = ask(&sender, Action::Inspect, false);
    let token = mcp_oauth_token::refresh(token, &cancel).unwrap();
    let (response, receiver) = mpsc::channel();
    updates
        .send(CredentialUpdate {
            endpoint: endpoint.clone(),
            credential: Secret::new(token.access_for(&endpoint).unwrap().expose().into()).unwrap(),
            response,
        })
        .unwrap();
    receiver
        .recv_timeout(Duration::from_secs(3))
        .unwrap()
        .unwrap();
    let result = ask(
        &sender,
        Action::Call {
            tool: "echo".into(),
            arguments: json!({"text":"TLS synthetic flow"}),
            expected_tool: report.tools[0].clone(),
        },
        true,
    );
    assert_eq!(
        result.call_result.unwrap()["content"][0]["text"],
        "TLS synthetic flow"
    );
    drop(updates);
    drop(sender);
    worker.join().unwrap();
    mcp_oauth_token::revoke(token, &cancel).unwrap();
    let refreshed = client()
        .post(&auth.token_endpoint)
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", "synthetic-refresh-new"),
            ("client_id", "synthetic-client"),
            ("resource", endpoint.as_str()),
        ])
        .send()
        .unwrap();
    assert_eq!(refreshed.status().as_u16(), 400);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&refreshed.bytes().unwrap()).unwrap()["error"],
        "invalid_grant"
    );
    let (sender, requests) = mpsc::channel();
    let worker_endpoint = endpoint.clone();
    let worker = thread::spawn(move || {
        let _trust = Trust::scoped(certificate);
        mcp_http::serve_http_authenticated(
            HttpConfig {
                endpoint: worker_endpoint,
            },
            Some(Secret::new("zi-sdk-synthetic-new".into()).unwrap()),
            Arc::new(AtomicBool::new(false)),
            requests,
        )
        .unwrap();
    });
    let (response, receiver) = mpsc::channel();
    sender
        .send(ConnectedRequest {
            action: Action::Inspect,
            manual_confirmed: false,
            response,
        })
        .unwrap();
    let error = receiver
        .recv_timeout(Duration::from_secs(20))
        .unwrap()
        .unwrap_err();
    assert!(error.contains("401"));
    assert!(!error.contains("zi-sdk-synthetic-new"));
    drop(sender);
    worker.join().unwrap();
    // SDK also saw the initial unauthenticated discovery GET in this TLS chain.
    assert_eq!(
        client()
            .post(url.join("/fixture/finish").unwrap())
            .send()
            .unwrap()
            .status()
            .as_u16(),
        200
    );
}
