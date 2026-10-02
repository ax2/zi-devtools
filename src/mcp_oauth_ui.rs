use crate::mcp_oauth::{self, AuthorizationMetadata, ResourceMetadata};
use eframe::egui;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    time::{Duration, Instant},
};

enum Reply {
    Address(mcp_oauth::DiscoveryAddress),
    Resource(ResourceMetadata),
    Authorization(AuthorizationMetadata),
}

enum LoginReply {
    Revoked(Result<(), String>),
    Registered(String, Vec<String>),
    Waiting,
    Exchanging,
    Finished(Result<Box<crate::mcp_oauth_token::TokenSet>, String>),
}

#[derive(Default)]
pub struct OAuthPanel {
    expanded: bool,
    endpoint: String,
    metadata_url: String,
    resource: Option<ResourceMetadata>,
    authorization: Option<AuthorizationMetadata>,
    selected: usize,
    receiver: Option<Receiver<Result<Reply, String>>>,
    message: String,
    enabled: bool,
    client_id: String,
    scopes: std::collections::BTreeSet<String>,
    login_receiver: Option<Receiver<LoginReply>>,
    login_cancel: Option<Arc<AtomicBool>>,
    token: Option<crate::mcp_oauth_token::TokenSet>,
    auto_refresh: bool,
    registered_client: bool,
    revoking: bool,
    revision: u64,
    refresh_deadline: Option<Instant>,
}
impl Drop for OAuthPanel {
    fn drop(&mut self) {
        self.clear();
    }
}
impl OAuthPanel {
    #[cfg(feature = "ui-preview")]
    pub fn preview_refresh(&mut self, endpoint: &str) {
        self.preview(endpoint);
        self.scopes.insert("tools:read".into());
        self.token = Some(crate::mcp_oauth_token::preview_token(
            endpoint,
            "https://auth.example.test/tenant",
            &self.client_id,
        ));
        self.message = "合成已登录状态 · 无实际服务或用户凭据".into();
    }
    #[cfg(test)]
    pub(crate) fn test_ready(due: bool) -> Self {
        let mut panel = tests::panel();
        panel.token = Some(if due {
            crate::mcp_oauth_token::fixture_due_token()
        } else {
            crate::mcp_oauth_token::fixture_token(false)
        });
        panel.revision = 1;
        panel.auto_refresh = due;
        panel
    }
    #[cfg(test)]
    pub(crate) fn test_pending(&mut self, result: Option<bool>) -> impl Sized + use<> {
        self.refresh_deadline = self
            .token
            .as_ref()
            .and_then(|token| token.remaining())
            .and_then(|remaining| Instant::now().checked_add(remaining));
        self.token = None;
        let (sender, receiver) = mpsc::channel();
        self.login_receiver = Some(receiver);
        if let Some(success) = result {
            let value = if success {
                Ok(Box::new(crate::mcp_oauth_token::fixture_token(false)))
            } else {
                Err("synthetic refresh failure".into())
            };
            sender.send(LoginReply::Finished(value)).unwrap();
        }
        sender
    }
    #[cfg(test)]
    pub(crate) fn test_expire_pending(&mut self) {
        self.refresh_deadline = Some(Instant::now() - Duration::from_secs(1));
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_registration(&mut self) {
        self.client_id.clear();
        self.authorization.as_mut().unwrap().registration_endpoint =
            Some("https://auth.example.test/register".into());
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_auto(&mut self) {
        self.auto_refresh = true;
    }
    pub fn clear(&mut self) {
        if let Some(cancel) = self.login_cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
        self.login_receiver = None;
        self.token = None;
        self.enabled = false;
        self.revoking = false;
        self.auto_refresh = false;
        if self.registered_client {
            self.client_id.clear();
        }
        self.registered_client = false;
        self.refresh_deadline = None;
    }
    pub fn enabled(&self) -> bool {
        self.enabled
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn pending(&self) -> bool {
        self.login_receiver.is_some()
    }
    pub fn refresh_due(&self) -> bool {
        self.enabled
            && self.auto_refresh
            && !self.pending()
            && self
                .token
                .as_ref()
                .and_then(|token| token.refresh_due())
                .is_some_and(|deadline| Instant::now() >= deadline)
    }
    pub fn expired(&self) -> bool {
        self.refresh_deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
            || self
                .token
                .as_ref()
                .and_then(|token| token.remaining())
                .is_some_and(|remaining| remaining.is_zero())
    }
    pub fn credential(&self, endpoint: &str) -> anyhow::Result<crate::credentials::Secret> {
        anyhow::ensure!(self.enabled, "当前未选择 OAuth 认证");
        let token = self
            .token
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("请先完成当前服务的登录或刷新"))?;
        anyhow::ensure!(
            token.bindings().2 == self.client_id.trim()
                && self
                    .authorization
                    .as_ref()
                    .is_some_and(|auth| auth.issuer == token.bindings().1),
            "登录结果与当前客户端或授权服务器不一致，请重新登录"
        );
        crate::credentials::Secret::new(token.access_for(endpoint)?.expose().to_owned())
    }
    fn start_login(&mut self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.registered_client,
            "临时注册结果只用于本次回调，请重新注册并登录"
        );
        self.start_login_mode(false)
    }
    fn start_login_mode(&mut self, dynamic: bool) -> anyhow::Result<()> {
        let auth = self
            .authorization
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("请先读取授权服务器信息"))?;
        let auth = auth.clone();
        if dynamic {
            anyhow::ensure!(
                auth.registration_endpoint.is_some(),
                "服务未声明开放注册地址"
            );
        }
        let resource = self.endpoint.clone();
        let scopes = self.scopes.iter().cloned().collect::<Vec<_>>();
        let callback = crate::mcp_oauth_callback::CallbackReceiver::bind()?;
        let tx = crate::mcp_oauth_login::Transaction::prepare(
            &auth,
            &resource,
            if dynamic {
                "registration-validation"
            } else {
                self.client_id.trim()
            },
            callback.redirect_uri(),
            &scopes,
        )?;
        self.clear();
        self.enabled = true;
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let (sender, receiver) = mpsc::channel();
        self.login_receiver = Some(receiver);
        self.login_cancel = Some(cancel);
        self.message = if dynamic {
            "正在注册本次公共客户端…最长 15 秒"
        } else {
            "正在打开系统浏览器…"
        }
        .into();
        std::thread::spawn(move || {
            let result = (|| -> anyhow::Result<_> {
                anyhow::ensure!(!worker_cancel.load(Ordering::Relaxed), "已取消登录");
                let tx = if dynamic {
                    let client = crate::mcp_oauth_registration::register(
                        &auth,
                        &resource,
                        callback.redirect_uri(),
                        &scopes,
                        &worker_cancel,
                    )?;
                    let tx = crate::mcp_oauth_login::Transaction::prepare(
                        &auth,
                        &resource,
                        &client.client_id,
                        callback.redirect_uri(),
                        &client.scopes,
                    )?;
                    sender
                        .send(LoginReply::Registered(client.client_id, client.scopes))
                        .map_err(|_| anyhow::anyhow!("注册任务已取消"))?;
                    tx
                } else {
                    tx
                };
                anyhow::ensure!(!worker_cancel.load(Ordering::Relaxed), "已取消授权任务");
                open::that_detached(tx.authorization_url().as_str())
                    .map_err(|_| anyhow::anyhow!("无法打开系统浏览器，请检查默认浏览器设置"))?;
                let _ = sender.send(LoginReply::Waiting);
                let grant = callback.receive(tx, &worker_cancel)?;
                let _ = sender.send(LoginReply::Exchanging);
                crate::mcp_oauth_token::exchange(grant, &worker_cancel)
            })()
            .map_err(|error| error.to_string());
            let _ = sender.send(LoginReply::Finished(result.map(Box::new)));
        });
        Ok(())
    }
    pub fn tick(&mut self, ctx: &egui::Context) {
        let reply = self.login_receiver.as_ref().map(Receiver::try_recv);
        match reply {
            Some(Ok(LoginReply::Revoked(result))) => {
                self.login_receiver = None;
                self.login_cancel = None;
                self.revoking = false;
                self.message = match result {
                    Ok(()) => "服务端已接受撤销请求；本机授权已清除".into(),
                    Err(error) => error,
                };
            }
            Some(Ok(LoginReply::Registered(client_id, scopes))) => {
                self.client_id = client_id;
                self.registered_client = true;
                self.scopes = scopes.into_iter().collect();
                self.message = "本次客户端注册已验证，正在打开浏览器…".into();
            }
            Some(Ok(LoginReply::Waiting)) => {
                self.message = "请在浏览器完成登录 · 最长等待 5 分钟".into()
            }
            Some(Ok(LoginReply::Exchanging)) => {
                self.message = "登录回调已验证，正在获取服务令牌…".into()
            }
            Some(Ok(LoginReply::Finished(result))) => {
                self.login_receiver = None;
                self.login_cancel = None;
                self.refresh_deadline = None;
                self.message = match result {
                    Ok(token) => {
                        self.token = Some(*token);
                        self.revision = self.revision.wrapping_add(1);
                        "已登录当前服务，可点击连接并保持".into()
                    }
                    Err(error) => error,
                };
            }
            Some(Err(mpsc::TryRecvError::Disconnected)) => {
                self.clear();
                self.message = "登录任务已结束，请重新登录".into();
            }
            _ => {}
        }
        if self.login_receiver.is_some() {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
    }
    pub fn can_revoke(&self) -> bool {
        self.enabled
            && !self.pending()
            && self.token.as_ref().is_some_and(|token| token.can_revoke())
    }
    fn take_revoke_token(&mut self) -> anyhow::Result<crate::mcp_oauth_token::TokenSet> {
        anyhow::ensure!(self.can_revoke(), "当前授权不支持公共客户端撤销");
        let token = self.token.as_ref().unwrap();
        anyhow::ensure!(
            token.bindings().2 == self.client_id.trim()
                && self
                    .authorization
                    .as_ref()
                    .is_some_and(|auth| auth.issuer == token.bindings().1)
                && mcp_oauth::canonical_resource(&self.endpoint)? == token.bindings().0,
            "授权目标已变化，不能撤销到其他服务"
        );
        let token = self.token.take().unwrap();
        self.clear();
        Ok(token)
    }
    pub fn start_revoke(&mut self) -> anyhow::Result<()> {
        let token = self.take_revoke_token()?;
        self.revoking = true;
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let (sender, receiver) = mpsc::channel();
        self.login_receiver = Some(receiver);
        self.login_cancel = Some(cancel);
        self.message = "本机授权已清除，正在请求服务端撤销…最长 15 秒".into();
        std::thread::spawn(move || {
            let result = crate::mcp_oauth_token::revoke(token, &worker_cancel)
                .map_err(|error| error.to_string());
            let _ = sender.send(LoginReply::Revoked(result));
        });
        Ok(())
    }
    pub fn has_refresh(&self) -> bool {
        self.enabled && self.token.as_ref().is_some_and(|token| token.has_refresh())
    }
    pub fn start_refresh(&mut self) -> anyhow::Result<()> {
        anyhow::ensure!(self.enabled, "当前未选择 OAuth 认证");
        anyhow::ensure!(self.login_receiver.is_none(), "授权任务正在进行");
        let token = self
            .token
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("当前没有可刷新的授权"))?;
        anyhow::ensure!(token.has_refresh(), "服务未提供刷新令牌，请重新登录");
        anyhow::ensure!(
            token.bindings().2 == self.client_id.trim()
                && self
                    .authorization
                    .as_ref()
                    .is_some_and(|auth| auth.issuer == token.bindings().1)
                && mcp_oauth::canonical_resource(&self.endpoint)? == token.bindings().0,
            "授权目标已变化，请重新登录"
        );
        self.refresh_deadline = token
            .remaining()
            .and_then(|remaining| Instant::now().checked_add(remaining));
        let token = self.token.take().unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let (sender, receiver) = mpsc::channel();
        self.login_receiver = Some(receiver);
        self.login_cancel = Some(cancel);
        self.message = "正在刷新当前服务授权…最长 15 秒".into();
        std::thread::spawn(move || {
            let result = crate::mcp_oauth_token::refresh(token, &worker_cancel)
                .map_err(|error| error.to_string());
            let _ = sender.send(LoginReply::Finished(result.map(Box::new)));
        });
        Ok(())
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview(&mut self, endpoint: &str) {
        self.endpoint = endpoint.into();
        self.metadata_url = mcp_oauth::resource_metadata_url(endpoint).unwrap();
        self.expanded = true;
        self.enabled = true;
        self.client_id = "synthetic-public-client".into();
        self.resource = Some(ResourceMetadata {
            resource: endpoint.into(),
            authorization_servers: vec!["https://auth.example.test/tenant".into()],
            scopes_supported: vec!["tools:read".into()],
            bearer_methods_supported: Some(vec!["header".into()]),
        });
        self.authorization = Some(AuthorizationMetadata {
            authorization_response_iss_parameter_supported: true,
            issuer: "https://auth.example.test/tenant".into(),
            authorization_endpoint: "https://auth.example.test/authorize".into(),
            token_endpoint: "https://auth.example.test/token".into(),
            registration_endpoint: None,
            revocation_endpoint: Some("https://auth.example.test/revoke".into()),
            revocation_endpoint_auth_methods_supported: Some(vec!["none".into()]),
            response_types_supported: vec!["code".into()],
            code_challenge_methods_supported: vec!["S256".into()],
            token_endpoint_auth_methods_supported: Some(vec!["none".into()]),
        });
        self.message = "合成授权信息 · 未连接服务 · 未登录".into();
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, endpoint: &str, allowed: bool) {
        if self.endpoint != endpoint {
            self.clear();
            self.client_id.clear();
            self.scopes.clear();
            self.endpoint = endpoint.into();
            self.metadata_url = mcp_oauth::resource_metadata_url(endpoint).unwrap_or_default();
            self.resource = None;
            self.authorization = None;
            self.selected = 0;
            self.receiver = None;
            self.message.clear();
        }
        self.tick(ui.ctx());
        if let Some(receiver) = &self.receiver {
            match receiver.try_recv() {
                Ok(reply) => {
                    self.receiver = None;
                    match reply {
                        Ok(Reply::Address(value)) => {
                            self.metadata_url = value.metadata_url;
                            self.resource = None;
                            self.authorization = None;
                            self.message = if value.advertised {
                                "服务返回了授权配置地址，请检查后主动读取"
                            } else {
                                "服务未返回配置地址提示，已生成默认地址；请按服务说明检查"
                            }
                            .into();
                        }
                        Ok(Reply::Resource(value)) => {
                            self.resource = Some(value);
                            self.authorization = None;
                            self.selected = 0;
                            self.message = "资源授权信息已读取，请选择授权服务器".into();
                        }
                        Ok(Reply::Authorization(value)) => {
                            self.authorization = Some(value);
                            self.message =
                                "授权服务器信息已读取，请填写公共客户端 ID 并选择权限".into();
                        }
                        Err(error) => self.message = error,
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.receiver = None;
                    self.message = "授权信息读取任务已结束".into();
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ui.ctx().request_repaint_after(Duration::from_millis(100))
                }
            }
        }
        egui::CollapsingHeader::new("OAuth 浏览器登录").id_salt("mcp-oauth-discovery").default_open(self.expanded).show(ui, |ui| {
            ui.weak("先检查服务与权限，再主动打开系统浏览器。授权只保留在内存；可使用预注册客户端或主动开放注册；可选择自动续期。");
            if let Ok(resource) = mcp_oauth::canonical_resource(endpoint) { ui.label(format!("目标资源：{resource}")); }
            egui::CollapsingHeader::new("服务发现与授权配置").id_salt("oauth-config-steps").default_open(self.authorization.is_none()).show(ui, |ui| {
            let active = allowed && self.receiver.is_none() && self.login_receiver.is_none();
            if ui.add_enabled(active, egui::Button::new("查找服务授权配置地址")).clicked() {
                self.clear();
                self.scopes.clear();
                self.resource = None;
                self.authorization = None;
                self.metadata_url.clear();
                let endpoint = endpoint.to_owned();
                let (sender, receiver) = mpsc::channel();
                self.receiver = Some(receiver);
                self.message = "正在查找服务授权配置地址…最长 10 秒".into();
                std::thread::spawn(move || {
                    let result = mcp_oauth::discover_address(&endpoint).map(Reply::Address).map_err(|e| e.to_string());
                    let _ = sender.send(result);
                });
            }
            ui.label("资源授权配置地址 · 可按服务说明修改");
            let active = allowed && self.receiver.is_none() && self.login_receiver.is_none();
            ui.add_enabled_ui(active, |ui| {
                if ui.add(egui::TextEdit::singleline(&mut self.metadata_url).desired_width(f32::INFINITY)).changed() {
                    self.clear();
                    self.scopes.clear();
                    self.resource = None;
                    self.authorization = None;
                }
                if ui.button("读取资源授权信息").clicked() {
                    self.clear();
                    self.scopes.clear();
                    let endpoint = endpoint.to_owned();
                    let url = self.metadata_url.clone();
                    self.resource = None;
                    self.authorization = None;
                    let (sender, receiver) = mpsc::channel();
                    self.receiver = Some(receiver);
                    self.message = "正在读取资源授权配置…最长 10 秒".into();
                    std::thread::spawn(move || {
                        let result = mcp_oauth::fetch_resource(&endpoint, &url).map(Reply::Resource).map_err(|e| e.to_string());
                        let _ = sender.send(result);
                    });
                }
            });
            if let Some(resource) = &self.resource {
                ui.label("选择授权服务器");
                for (index, issuer) in resource.authorization_servers.iter().enumerate() {
                    if ui.add_enabled(active, egui::RadioButton::new(self.selected == index, issuer)).clicked() {
                        self.selected = index;
                        self.authorization = None;
                        self.token = None;
                        self.client_id.clear();
                        self.scopes.clear();
                    }
                }
                if !resource.scopes_supported.is_empty() {
                    ui.label(format!("服务声明的权限（尚未申请）：{}", resource.scopes_supported.join("、")));
                }
                if let Some(issuer) = resource.authorization_servers.get(self.selected) {
                    if let Ok(url) = mcp_oauth::authorization_metadata_url(issuer) { ui.label(format!("将读取：{url}")); }
                    if ui.add_enabled(active, egui::Button::new("读取所选授权服务器信息")).clicked() {
                        let issuer = issuer.clone();
                        self.token = None;
                        let (sender, receiver) = mpsc::channel();
                        self.receiver = Some(receiver);
                        self.authorization = None;
                        self.message = "正在读取所选授权服务器配置…最长 10 秒".into();
                        std::thread::spawn(move || {
                            let result = mcp_oauth::fetch_authorization(&issuer).map(Reply::Authorization).map_err(|e| e.to_string());
                            let _ = sender.send(result);
                        });
                    }
                }
            }
            });
            if let Some(auth) = &self.authorization {
                ui.separator();
                egui::CollapsingHeader::new("检查授权服务配置").show(ui, |ui| {
                    ui.label(format!("授权服务器：{}", auth.issuer));
                    ui.label(format!("登录地址：{}", auth.authorization_endpoint));
                    ui.label(format!("令牌地址：{}", auth.token_endpoint));
                    ui.label(if auth.code_challenge_methods_supported.iter().any(|v| v == "S256") { "支持 PKCE S256" } else { "未声明 PKCE S256，无法登录" });
                    if let Some(url) = &auth.registration_endpoint { ui.label(format!("开放注册地址：{url} · 注册并登录时将主动请求")); }
                });
                let active = allowed && self.receiver.is_none() && self.login_receiver.is_none();
                ui.add_enabled_ui(active, |ui| {
                    if ui.checkbox(&mut self.enabled, "本次连接使用 OAuth 登录").changed() && !self.enabled { self.token = None; }
                    egui::CollapsingHeader::new("客户端 ID 与申请权限").id_salt(("oauth-client-fields", self.token.is_some())).default_open(self.token.is_none()).show(ui, |ui| {
                    ui.label("公共客户端 ID · 预注册时填写，开放注册可留空；认证方式须为 none");
                    ui.small("回调路径：/oauth/callback/zi-devtools；服务须允许 127.0.0.1 的动态端口");
                    if ui.add(egui::TextEdit::singleline(&mut self.client_id).char_limit(512).desired_width(f32::INFINITY)).changed() { self.token = None; self.registered_client = false; }
                    if let Some(resource) = &self.resource {
                        ui.label(format!("申请权限 · 已选 {} 项，最多 16 项", self.scopes.len()));
                        for scope in &resource.scopes_supported {
                            let mut checked = self.scopes.contains(scope);
                            if ui.checkbox(&mut checked, scope).changed() {
                                if checked { self.scopes.insert(scope.clone()); } else { self.scopes.remove(scope); }
                                self.token = None;
                            }
                        }
                        if resource.scopes_supported.is_empty() { ui.weak("服务未声明可选权限，请在浏览器确认实际授权范围。"); }
                    }
                    });
                });
                let can_register = auth.registration_endpoint.is_some();
                if let Some(url) = &auth.registration_endpoint {
                    ui.small(format!("开放注册目标：{url}"));
                    ui.small("注册将创建服务端客户端记录；本机不保存，失败或取消不自动删除。每次注册并登录创建新客户端。");
                }
                ui.horizontal_wrapped(|ui| {
                    if ui.add_enabled(active && self.enabled && !self.registered_client && !self.client_id.trim().is_empty() && self.scopes.len() <= 16, egui::Button::new("在浏览器登录")).clicked() {
                        if let Err(error) = self.start_login() { self.message = error.to_string(); }
                    }
                    if ui.add_enabled(active && self.enabled && can_register && self.scopes.len() <= 16, egui::Button::new("注册并登录")).clicked() {
                        if let Err(error) = self.start_login_mode(true) { self.message = error.to_string(); }
                    }
                    if ui.add_enabled(active && self.enabled && self.token.as_ref().is_some_and(|token| token.has_refresh()), egui::Button::new("刷新本次授权")).clicked() {
                        if let Err(error) = self.start_refresh() { self.message = error.to_string(); }
                    }
                    if ui.add_enabled(active && self.can_revoke(), egui::Button::new("撤销服务端授权")).clicked() {
                        if let Err(error) = self.start_revoke() { self.message = error.to_string(); }
                    }
                    if ui.add_enabled(allowed && (self.login_receiver.is_some() || self.token.is_some()), egui::Button::new(if self.login_receiver.is_some() { "取消授权任务" } else { "清除本次授权" })).clicked() {
                        let revoking = self.revoking;
                        self.clear();
                        self.message = if revoking { "本机授权已清除；撤销任务已取消，服务端结果未知" } else { "本次授权已在本机清除；未向服务端撤销，已打开的浏览器页面可手动关闭" }.into();
                    }
                });
                if let Some(target) = self.token.as_ref().and_then(|token| token.revocation_target()) { ui.small(format!("撤销目标：{target}")); }
                ui.small("清除仅移除本机授权；撤销会联系服务端，不删除客户端注册记录。");
                if self.registered_client { ui.small("临时注册 ID 仅用于本次回调；再次登录需重新注册。服务端记录可能保留，取消不会自动删除。"); }
                if let Some(token) = &self.token {
                    if !token.can_revoke() { ui.small("服务未声明公共客户端撤销入口；本机清除不会撤销服务端授权。"); }
                    ui.label(match token.remaining() { Some(duration) if duration.is_zero() => "访问令牌已过期，请重新登录".into(), Some(duration) => format!("已登录 · 剩余有效期约 {} 分钟", duration.as_secs().div_ceil(60)), None => "已登录 · 服务未提供有效期".into() });
                    ui.small("只用于当前服务；刷新失败或取消须重新登录。授权不会持久保存。");
                    ui.ctx().request_repaint_after(Duration::from_secs(1));
                }
            }
            if self.enabled {
                let eligible = self.token.as_ref().and_then(|token| token.refresh_due()).is_some();
                ui.add_enabled_ui(eligible && !self.pending(), |ui| { ui.checkbox(&mut self.auto_refresh, "连接期间自动续期授权"); });
                if !eligible && !self.pending() { ui.small("自动续期需要刷新令牌和超过 30 秒的明确有效期；不猜测未知期限。"); }
                if self.pending() && self.auto_refresh { ui.small("正在续期，新操作暂停；失败或取消将断开连接。"); }
            }
            if self.login_receiver.is_some() { ui.spinner(); }
            if self.receiver.is_some() { ui.spinner(); }
            if !self.message.is_empty() { ui.label(&self.message); }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    pub(super) fn panel() -> OAuthPanel {
        OAuthPanel {
            endpoint: "https://mcp.example.test/mcp".into(), enabled: true,
            client_id: "synthetic-client".into(),
            authorization: Some(serde_json::from_value(serde_json::json!({
                "issuer":"https://auth.example.test", "authorization_endpoint":"https://auth.example.test/authorize",
                "token_endpoint":"https://auth.example.test/token", "response_types_supported":["code"]
            })).unwrap()),
            expanded: false,
            metadata_url: String::new(),
            resource: None,
            selected: 0,
            receiver: None,
            message: String::new(),
            scopes: Default::default(),
            login_receiver: None,
            login_cancel: None,
            token: None,
            auto_refresh: false,
            registered_client: false,
            revoking: false,
            revision: 0,
            refresh_deadline: None,
        }
    }
    fn poll(panel: &mut OAuthPanel) {
        let ctx = egui::Context::default();
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| panel.tick(ui.ctx()));
        });
    }
    #[test]
    fn revocation_moves_credentials_out_before_job_and_never_restores_on_terminal_result() {
        for success in [true, false] {
            let mut panel = panel();
            panel.token = Some(crate::mcp_oauth_token::fixture_revoke_token());
            panel.auto_refresh = true;
            let token = panel.take_revoke_token().unwrap();
            assert!(token.can_revoke());
            assert!(panel.token.is_none() && !panel.enabled && !panel.auto_refresh);
            let (sender, receiver) = mpsc::channel();
            panel.login_receiver = Some(receiver);
            panel.revoking = true;
            sender
                .send(LoginReply::Revoked(if success {
                    Ok(())
                } else {
                    Err("synthetic failure".into())
                }))
                .unwrap();
            poll(&mut panel);
            assert!(panel.token.is_none() && !panel.enabled && !panel.pending() && !panel.revoking);
            assert!(panel.credential("https://mcp.example.test/mcp").is_err());
        }
        let mut panel = panel();
        panel.token = Some(crate::mcp_oauth_token::fixture_revoke_token());
        panel.endpoint = "https://other.example.test/mcp".into();
        assert!(panel.take_revoke_token().is_err());
        assert!(panel.token.is_some());
    }
    #[test]
    fn registration_result_narrows_scopes_and_cannot_be_reused_after_clear() {
        let mut panel = panel();
        panel.scopes.insert("tools:read".into());
        panel.scopes.insert("tools:write".into());
        let (sender, receiver) = mpsc::channel();
        panel.login_receiver = Some(receiver);
        sender
            .send(LoginReply::Registered(
                "synthetic-dynamic-id".into(),
                vec!["tools:read".into()],
            ))
            .unwrap();
        poll(&mut panel);
        assert!(panel.registered_client);
        assert_eq!(panel.scopes.len(), 1);
        assert_eq!(panel.client_id, "synthetic-dynamic-id");
        assert!(panel.start_login().is_err());
        panel.clear();
        assert!(panel.client_id.is_empty());
        assert!(sender.send(LoginReply::Waiting).is_err());
    }
    #[test]
    fn cancelled_old_login_cannot_deliver_into_new_login() {
        let mut panel = panel();
        let cancel = Arc::new(AtomicBool::new(false));
        let (old_sender, old_receiver) = mpsc::channel();
        panel.login_cancel = Some(cancel.clone());
        panel.login_receiver = Some(old_receiver);
        panel.clear();
        assert!(cancel.load(Ordering::Relaxed));
        assert!(
            old_sender
                .send(LoginReply::Finished(Ok(Box::new(
                    crate::mcp_oauth_token::fixture_token(false)
                ))))
                .is_err()
        );
        let (new_sender, new_receiver) = mpsc::channel();
        panel.login_receiver = Some(new_receiver);
        panel.enabled = true;
        new_sender.send(LoginReply::Waiting).unwrap();
        poll(&mut panel);
        assert!(panel.token.is_none() && panel.message.contains("浏览器"));
        new_sender.send(LoginReply::Exchanging).unwrap();
        poll(&mut panel);
        assert!(panel.message.contains("获取"));
        new_sender
            .send(LoginReply::Finished(Ok(Box::new(
                crate::mcp_oauth_token::fixture_token(false),
            ))))
            .unwrap();
        poll(&mut panel);
        assert!(panel.login_receiver.is_none());
        assert!(panel.credential("https://mcp.example.test/mcp").is_ok());
    }
    #[test]
    fn resource_client_issuer_expiry_and_mode_all_bind_connection_credential() {
        let mut panel = panel();
        panel.token = Some(crate::mcp_oauth_token::fixture_token(false));
        assert!(panel.credential("https://mcp.example.test/mcp").is_ok());
        assert!(panel.credential("https://other.example.test/mcp").is_err());
        panel.client_id = "different-client".into();
        assert!(panel.credential("https://mcp.example.test/mcp").is_err());
        panel.client_id = "synthetic-client".into();
        panel.authorization.as_mut().unwrap().issuer = "https://other.example.test".into();
        assert!(panel.credential("https://mcp.example.test/mcp").is_err());
        panel.authorization.as_mut().unwrap().issuer = "https://auth.example.test".into();
        panel.token = Some(crate::mcp_oauth_token::fixture_token(true));
        assert!(panel.credential("https://mcp.example.test/mcp").is_err());
        panel.token = Some(crate::mcp_oauth_token::fixture_token(false));
        panel.enabled = false;
        assert!(panel.credential("https://mcp.example.test/mcp").is_err());
    }
    #[test]
    fn endpoint_edit_resets_pending_login_and_authorization() {
        let mut panel = panel();
        panel.token = Some(crate::mcp_oauth_token::fixture_token(false));
        let cancel = Arc::new(AtomicBool::new(false));
        let (_sender, receiver) = mpsc::channel();
        panel.login_cancel = Some(cancel.clone());
        panel.login_receiver = Some(receiver);
        let ctx = egui::Context::default();
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                panel.ui(ui, "https://other.example.test/mcp", true)
            });
        });
        assert!(cancel.load(Ordering::Relaxed));
        assert!(
            panel.token.is_none()
                && panel.login_receiver.is_none()
                && panel.authorization.is_none()
        );
        assert!(!panel.enabled && panel.client_id.is_empty());
    }
}
