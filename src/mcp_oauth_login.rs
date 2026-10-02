//! PKCE transaction and callback validation; UI/browser/token exchange is not wired yet.
use crate::{
    credentials::Secret,
    mcp_oauth::{self, AuthorizationMetadata},
};
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use reqwest::Url;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

fn random_secret() -> Result<Secret> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| anyhow::anyhow!("无法生成登录随机参数"))?;
    let secret = Secret::new(URL_SAFE_NO_PAD.encode(bytes));
    bytes.fill(0);
    secret
}
fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}
fn equal_state(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0u8, |difference, (a, b)| difference | (a ^ b))
        == 0
}

/// No Debug/Serialize; all server and resource bindings are frozen for this transaction.
pub struct Transaction {
    authorization_url: Url,
    issuer: String,
    token_endpoint: Url,
    redirect: Url,
    resource: String,
    client_id: String,
    state: Secret,
    verifier: Option<Secret>,
    require_issuer: bool,
    deadline: Instant,
    consumed: bool,
}

/// Can only be constructed after a validated callback. No Debug/Serialize.
pub struct CodeGrant {
    code: Secret,
    verifier: Secret,
    token_endpoint: Url,
    redirect: String,
    resource: String,
    client_id: String,
}
impl CodeGrant {
    pub fn code(&self) -> &Secret {
        &self.code
    }
    pub fn verifier(&self) -> &Secret {
        &self.verifier
    }
    pub fn bindings(&self) -> (&Url, &str, &str, &str) {
        (
            &self.token_endpoint,
            &self.redirect,
            &self.resource,
            &self.client_id,
        )
    }
}

impl Transaction {
    pub fn prepare(
        metadata: &AuthorizationMetadata,
        resource: &str,
        client_id: &str,
        redirect: &str,
        scopes: &[String],
    ) -> Result<Self> {
        mcp_oauth::secure_url(&metadata.issuer, false)?;
        ensure!(
            metadata
                .code_challenge_methods_supported
                .iter()
                .any(|v| v == "S256"),
            "授权服务未声明 PKCE S256"
        );
        ensure!(
            metadata
                .response_types_supported
                .iter()
                .any(|v| v == "code"),
            "授权服务未声明授权码流程"
        );
        ensure!(
            metadata
                .token_endpoint_auth_methods_supported
                .as_ref()
                .is_some_and(|methods| methods.iter().any(|v| v == "none")),
            "授权服务未声明公共客户端认证方式 none"
        );
        ensure!(
            !client_id.is_empty()
                && client_id.len() <= 512
                && !client_id.chars().any(char::is_control),
            "客户端 ID 为空、过长或含控制字符"
        );
        ensure!(
            scopes.len() <= 16
                && scopes.iter().all(|scope| !scope.is_empty()
                    && scope.len() <= 256
                    && scope.bytes().all(|b| b == 0x21
                        || (0x23..=0x5b).contains(&b)
                        || (0x5d..=0x7e).contains(&b))),
            "登录权限格式无效或数量过多"
        );
        let unique: std::collections::BTreeSet<_> = scopes.iter().collect();
        ensure!(unique.len() == scopes.len(), "登录权限包含重复项");
        ensure!(
            redirect.len() <= 2048 && !redirect.chars().any(char::is_control),
            "登录回调地址过长或含控制字符"
        );
        let redirect = Url::parse(redirect).context("登录回调地址无效")?;
        ensure!(
            redirect.scheme() == "http"
                && redirect.host_str() == Some("127.0.0.1")
                && redirect.port().is_some_and(|port| port > 0)
                && redirect.username().is_empty()
                && redirect.password().is_none()
                && redirect.query().is_none()
                && redirect.fragment().is_none()
                && redirect.path().starts_with("/oauth/callback/")
                && redirect.path().len() >= 24
                && redirect.path().len() <= 256,
            "登录回调须为本机随机端口的专用地址"
        );
        let resource = mcp_oauth::canonical_resource(resource)?;
        let mut authorization_url = mcp_oauth::secure_url(&metadata.authorization_endpoint, true)?;
        let token_endpoint = mcp_oauth::secure_url(&metadata.token_endpoint, true)?;
        let reserved = [
            "response_type",
            "client_id",
            "redirect_uri",
            "scope",
            "state",
            "resource",
            "code_challenge",
            "code_challenge_method",
            "request",
            "request_uri",
        ];
        for url in [&authorization_url, &token_endpoint] {
            ensure!(
                !url.query_pairs()
                    .any(|(key, _)| reserved.contains(&key.as_ref())),
                "授权端点已包含冲突的登录参数"
            );
        }
        let state = random_secret()?;
        let verifier = random_secret()?;
        {
            let mut query = authorization_url.query_pairs_mut();
            query
                .append_pair("response_type", "code")
                .append_pair("client_id", client_id)
                .append_pair("redirect_uri", redirect.as_str())
                .append_pair("resource", &resource)
                .append_pair("state", state.expose())
                .append_pair("code_challenge_method", "S256")
                .append_pair("code_challenge", &challenge(verifier.expose()));
            if !scopes.is_empty() {
                query.append_pair("scope", &scopes.join(" "));
            }
        }
        ensure!(
            authorization_url.as_str().len() <= 8192,
            "登录请求地址超过 8 KiB"
        );
        Ok(Self {
            authorization_url,
            issuer: metadata.issuer.clone(),
            token_endpoint,
            redirect,
            resource,
            client_id: client_id.into(),
            state,
            verifier: Some(verifier),
            require_issuer: metadata.authorization_response_iss_parameter_supported,
            deadline: Instant::now() + Duration::from_secs(300),
            consumed: false,
        })
    }

    /// For the authorized browser opening only; never log or archive this URL.
    pub fn authorization_url(&self) -> &Url {
        &self.authorization_url
    }

    pub fn cancel(&mut self) {
        self.consumed = true;
        self.verifier = None;
    }

    pub(crate) fn finished(&self) -> bool {
        self.consumed || Instant::now() >= self.deadline
    }

    pub fn accept_callback(&mut self, source: &str) -> Result<CodeGrant> {
        ensure!(
            !self.consumed && Instant::now() < self.deadline,
            "本次登录已结束或超过 5 分钟"
        );
        ensure!(
            source.len() <= 8192 && !source.chars().any(char::is_control),
            "登录回调过长或含控制字符"
        );
        let url = Url::parse(source).context("登录回调无效")?;
        ensure!(
            url.origin() == self.redirect.origin()
                && url.path() == self.redirect.path()
                && url.username().is_empty()
                && url.password().is_none()
                && url.fragment().is_none(),
            "登录回调目标与本次登录不一致"
        );
        let mut fields = BTreeMap::new();
        let raw_query = url.query().unwrap_or_default().as_bytes();
        for (index, byte) in raw_query.iter().enumerate() {
            if *byte == b'%' {
                ensure!(
                    raw_query.get(index + 1).is_some_and(u8::is_ascii_hexdigit)
                        && raw_query.get(index + 2).is_some_and(u8::is_ascii_hexdigit),
                    "登录回调包含无效编码"
                );
            }
        }
        for (key, value) in url.query_pairs() {
            ensure!(
                !key.contains('\u{fffd}')
                    && !value.contains('\u{fffd}')
                    && fields.len() < 16
                    && fields
                        .insert(key.into_owned(), value.into_owned())
                        .is_none(),
                "登录回调参数过多或重复"
            );
        }
        let state = fields.get("state").context("登录回调缺少 state")?;
        ensure!(
            equal_state(state.as_bytes(), self.state.expose().as_bytes()),
            "登录回调 state 与本次登录不一致"
        );
        if let Some(issuer) = fields.get("iss") {
            ensure!(issuer == &self.issuer, "登录回调 issuer 与所选服务不一致");
        } else {
            ensure!(!self.require_issuer, "登录回调缺少授权服务声明的 issuer");
        }
        ensure!(
            !(fields.contains_key("code") && fields.contains_key("error")),
            "登录回调同时包含授权码和错误"
        );
        if let Some(error) = fields.get("error") {
            self.cancel();
            if error == "access_denied" {
                bail!("用户取消了本次登录");
            }
            bail!("授权服务返回登录错误");
        }
        let code = fields.remove("code").context("登录回调缺少授权码")?;
        ensure!(
            !code.is_empty()
                && code.len() <= 2048
                && code.bytes().all(|b| (0x21..=0x7e).contains(&b)),
            "授权码格式无效"
        );
        let code = Secret::new(code)?;
        let verifier = self.verifier.take().context("本次登录已结束")?;
        self.consumed = true;
        Ok(CodeGrant {
            code,
            verifier,
            token_endpoint: self.token_endpoint.clone(),
            redirect: self.redirect.to_string(),
            resource: self.resource.clone(),
            client_id: self.client_id.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn metadata() -> AuthorizationMetadata {
        serde_json::from_value(serde_json::json!({
            "issuer":"https://auth.example.test/tenant",
            "authorization_endpoint":"https://auth.example.test/authorize",
            "token_endpoint":"https://auth.example.test/token",
            "response_types_supported":["code"], "code_challenge_methods_supported":["S256"],
            "token_endpoint_auth_methods_supported":["none"],
            "authorization_response_iss_parameter_supported":true
        }))
        .unwrap()
    }
    fn transaction() -> Transaction {
        Transaction::prepare(
            &metadata(),
            "https://mcp.example.test/mcp",
            "synthetic-public-client",
            "http://127.0.0.1:50123/oauth/callback/synthetic-fixture",
            &["tools:read".into()],
        )
        .unwrap()
    }
    fn callback(tx: &Transaction, state: &str, issuer: &str) -> String {
        let mut url = tx.redirect.clone();
        url.query_pairs_mut()
            .append_pair("code", "synthetic-code")
            .append_pair("state", state)
            .append_pair("iss", issuer);
        url.to_string()
    }
    #[test]
    fn pkce_matches_rfc7636_vector_and_binds_resource_without_verifier_in_url() {
        assert_eq!(
            challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        let tx = transaction();
        let other = transaction();
        assert!(!equal_state(
            tx.state.expose().as_bytes(),
            other.state.expose().as_bytes()
        ));
        let fields: BTreeMap<_, _> = tx.authorization_url.query_pairs().collect();
        assert_eq!(fields["resource"], "https://mcp.example.test/mcp");
        assert_eq!(fields["code_challenge_method"], "S256");
        assert_eq!(tx.verifier.as_ref().unwrap().expose().len(), 43);
        assert!(
            !tx.authorization_url
                .as_str()
                .contains(tx.verifier.as_ref().unwrap().expose())
        );
    }
    #[test]
    fn correct_callback_produces_one_grant_and_replay_is_rejected() {
        let mut tx = transaction();
        let url = callback(&tx, tx.state.expose(), &tx.issuer);
        let grant = tx.accept_callback(&url).unwrap();
        assert!(grant.code().expose() == "synthetic-code");
        assert!(grant.verifier().expose().len() == 43);
        let (endpoint, redirect, resource, client) = grant.bindings();
        assert_eq!(endpoint.as_str(), "https://auth.example.test/token");
        assert_eq!(
            redirect,
            "http://127.0.0.1:50123/oauth/callback/synthetic-fixture"
        );
        assert_eq!(resource, "https://mcp.example.test/mcp");
        assert_eq!(client, "synthetic-public-client");
        assert!(tx.accept_callback(&url).is_err());
    }
    #[test]
    fn invalid_callback_cannot_consume_valid_pending_login() {
        let mut tx = transaction();
        let valid = callback(&tx, tx.state.expose(), &tx.issuer);
        for invalid in [
            callback(&tx, "wrong-state", &tx.issuer),
            callback(&tx, tx.state.expose(), "https://other.example.test"),
            valid.replace("50123", "50124"),
            valid.replace("synthetic-fixture", "other-fixture"),
            format!("{valid}&code=second"),
            format!("{valid}&error=access_denied"),
            format!("{valid}#fragment"),
            valid.replace("synthetic-code", "%GG"),
            valid.replace("synthetic-code", "%FF"),
        ] {
            assert!(tx.accept_callback(&invalid).is_err());
        }
        assert!(tx.accept_callback(&valid).is_ok());
    }
    #[test]
    fn missing_issuer_cancellation_and_deadline_reject_without_secret_errors() {
        let mut tx = transaction();
        let mut url = tx.redirect.clone();
        url.query_pairs_mut()
            .append_pair("state", tx.state.expose())
            .append_pair("error", "access_denied")
            .append_pair("error_description", "synthetic-sensitive-description");
        assert!(tx.accept_callback(url.as_str()).is_err());
        url.query_pairs_mut().append_pair("iss", &tx.issuer);
        let error = tx.accept_callback(url.as_str()).err().unwrap().to_string();
        assert_eq!(error, "用户取消了本次登录");
        assert!(tx.accept_callback(url.as_str()).is_err());
        let mut tx = transaction();
        let valid = callback(&tx, tx.state.expose(), &tx.issuer);
        tx.deadline = Instant::now() - Duration::from_secs(1);
        assert!(tx.accept_callback(&valid).is_err());
        let mut tx = transaction();
        tx.cancel();
        assert!(tx.accept_callback(&valid).is_err());
    }
    #[test]
    fn insecure_redirect_conflicting_params_and_unsupported_pkce_are_rejected() {
        let mut config = metadata();
        for redirect in [
            "http://localhost:50123/oauth/callback/synthetic-fixture",
            "http://192.168.1.2:50123/oauth/callback/synthetic-fixture",
            "http://127.0.0.1:0/oauth/callback/synthetic-fixture",
        ] {
            assert!(
                Transaction::prepare(
                    &config,
                    "https://mcp.example.test/mcp",
                    "client",
                    redirect,
                    &[]
                )
                .is_err()
            );
        }
        config.authorization_endpoint =
            "https://auth.example.test/authorize?state=preexisting".into();
        assert!(
            Transaction::prepare(
                &config,
                "https://mcp.example.test/mcp",
                "client",
                "http://127.0.0.1:50123/oauth/callback/synthetic-fixture",
                &[]
            )
            .is_err()
        );
        config = metadata();
        config.code_challenge_methods_supported = vec!["plain".into()];
        assert!(
            Transaction::prepare(
                &config,
                "https://mcp.example.test/mcp",
                "client",
                "http://127.0.0.1:50123/oauth/callback/synthetic-fixture",
                &[]
            )
            .is_err()
        );
    }
}
