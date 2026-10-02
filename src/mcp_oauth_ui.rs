use crate::mcp_oauth::{self, AuthorizationMetadata, ResourceMetadata};
use eframe::egui;
use std::{
    sync::mpsc::{self, Receiver},
    time::Duration,
};

enum Reply {
    Resource(ResourceMetadata),
    Authorization(AuthorizationMetadata),
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
}
impl OAuthPanel {
    #[cfg(feature = "ui-preview")]
    pub fn preview(&mut self, endpoint: &str) {
        self.endpoint = endpoint.into();
        self.metadata_url = mcp_oauth::resource_metadata_url(endpoint).unwrap();
        self.expanded = true;
        self.resource = Some(ResourceMetadata {
            resource: endpoint.into(),
            authorization_servers: vec!["https://auth.example.test/tenant".into()],
            scopes_supported: vec!["tools:read".into()],
            bearer_methods_supported: Some(vec!["header".into()]),
        });
        self.authorization = Some(AuthorizationMetadata {
            issuer: "https://auth.example.test/tenant".into(),
            authorization_endpoint: "https://auth.example.test/authorize".into(),
            token_endpoint: "https://auth.example.test/token".into(),
            registration_endpoint: None,
            response_types_supported: vec!["code".into()],
            code_challenge_methods_supported: vec!["S256".into()],
            token_endpoint_auth_methods_supported: Some(vec!["none".into()]),
        });
        self.message = "合成授权信息 · 未连接服务 · 未登录".into();
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, endpoint: &str, allowed: bool) {
        if self.endpoint != endpoint {
            self.endpoint = endpoint.into();
            self.metadata_url = mcp_oauth::resource_metadata_url(endpoint).unwrap_or_default();
            self.resource = None;
            self.authorization = None;
            self.selected = 0;
            self.receiver = None;
            self.message.clear();
        }
        if let Some(receiver) = &self.receiver {
            match receiver.try_recv() {
                Ok(reply) => {
                    self.receiver = None;
                    match reply {
                        Ok(Reply::Resource(value)) => {
                            self.resource = Some(value);
                            self.authorization = None;
                            self.selected = 0;
                            self.message = "资源授权信息已读取，请选择授权服务器".into();
                        }
                        Ok(Reply::Authorization(value)) => {
                            self.authorization = Some(value);
                            self.message = "授权服务器信息已读取；浏览器登录流程尚未接入".into();
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
        egui::CollapsingHeader::new("OAuth 登录准备 · 查看授权信息").id_salt("mcp-oauth-discovery").default_open(self.expanded).show(ui, |ui| {
            ui.weak("逐步读取公开 HTTPS 配置，不发送令牌、不自动登录。当前尚未接入 401 地址发现、浏览器授权与自动刷新。");
            if let Ok(resource) = mcp_oauth::canonical_resource(endpoint) { ui.label(format!("目标资源：{resource}")); }
            ui.label("资源授权配置地址 · 可按服务说明修改");
            let active = allowed && self.receiver.is_none();
            ui.add_enabled_ui(active, |ui| {
                if ui.add(egui::TextEdit::singleline(&mut self.metadata_url).desired_width(f32::INFINITY)).changed() {
                    self.resource = None;
                    self.authorization = None;
                }
                if ui.button("读取资源授权信息").clicked() {
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
                    }
                }
                if !resource.scopes_supported.is_empty() {
                    ui.label(format!("服务声明的权限（尚未申请）：{}", resource.scopes_supported.join("、")));
                }
                if let Some(issuer) = resource.authorization_servers.get(self.selected) {
                    if let Ok(url) = mcp_oauth::authorization_metadata_url(issuer) { ui.label(format!("将读取：{url}")); }
                    if ui.add_enabled(active, egui::Button::new("读取所选授权服务器信息")).clicked() {
                        let issuer = issuer.clone();
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
            if let Some(auth) = &self.authorization {
                ui.separator();
                ui.label(format!("授权服务器：{}", auth.issuer));
                ui.label(format!("登录地址：{}", auth.authorization_endpoint));
                ui.label(format!("令牌地址：{}", auth.token_endpoint));
                ui.label(if auth.code_challenge_methods_supported.iter().any(|v| v == "S256") { "支持 PKCE S256" } else { "未声明 PKCE S256，后续登录不能使用此配置" });
                ui.label(if auth.response_types_supported.iter().any(|v| v == "code") { "支持授权码流程" } else { "未声明授权码流程" });
                if let Some(url) = &auth.registration_endpoint { ui.label(format!("客户端注册地址：{url}")); }
            }
            if self.receiver.is_some() { ui.spinner(); }
            if !self.message.is_empty() { ui.label(&self.message); }
        });
    }
}
