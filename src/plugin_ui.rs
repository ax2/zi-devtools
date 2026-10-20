use crate::plugins::{self, Adapter, Store};
mod prompts;
use crate::tasks::{Job, Phase, Row};
use eframe::egui;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::mpsc::{self, Receiver},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Default)]
struct Draft {
    input: String,
    output: String,
    output_completed: bool,
    output_handoffable: bool,
    probe_mode: bool,
    probe_previous_input: Option<String>,
    model: String,
    endpoint: String,
    settings_key: String,
    settings_message: String,
    profile_id: Option<String>,
    profile_name: String,
    profile_editor: bool,
    profile_delete: bool,
    discovery_open: bool,
    models: Vec<String>,
    models_source: String,
    discovery_message: String,
    chat_mode: bool,
    conversation: crate::conversation::Conversation,
    pending_chat: Option<(String, String)>,
    chat_message: String,
    stream_mode: bool,
    attachments: crate::chat_attachments::State,
    export_json: bool,
    export_path: String,
    import_path: String,
    import_preview: Option<crate::conversation::Conversation>,
    import_open: bool,
    library_name: String,
    library_entries: Vec<crate::chat_library::Entry>,
    library_selected: Option<String>,
    library_preview: Option<crate::conversation::Conversation>,
    library_loaded: bool,
    library_open: bool,
    library_skipped: usize,
    library_truncated: bool,
    library_delete_confirm: bool,
    prompts: prompts::State,
}
impl Draft {
    fn bind_conversation(&mut self, binding: String) {
        let changed = self.conversation.binding() != binding;
        let had_context = self.conversation.bind(binding);
        if changed {
            self.import_preview = None;
            self.library_preview = None;
        }
        if had_context {
            self.chat_message = "连接、模型或工具定义已改变，旧会话已清空".into();
        }
    }
    fn apply_prompt(&mut self, action: prompts::Apply, tool_id: &str, settings_key: &str) {
        self.input = action.text;
        self.output.clear();
        if let Some(options) = action.options {
            if options.tool_id != tool_id {
                self.chat_message = "模板来源工具不匹配；仅填入正文，运行参数未更改".into();
                return;
            }
            self.model = options.model;
            self.stream_mode = options.stream;
            self.chat_mode = options.multi_turn;
            self.bind_conversation(format!("{settings_key}:{}", self.model));
            if !self.chat_mode {
                self.conversation.clear();
            }
            self.chat_message =
                "模板正文与参数已填入；模型或多轮设置不兼容时已清空旧上下文，尚未发送请求".into();
        } else {
            self.chat_message = "模板正文已填入输入框；尚未发送请求".into();
        }
    }
}
#[derive(Clone)]
struct StreamControl {
    cancel: Arc<AtomicBool>,
    partial: Arc<Mutex<String>>,
}

fn show_tool_aliases(ui: &mut egui::Ui, plugin_id: &str, tool: &plugins::PluginTool, scope: &str) {
    if tool.aliases.is_empty() {
        return;
    }
    egui::CollapsingHeader::new(format!(
        "{} · 搜索别名（{}）",
        tool.name,
        tool.aliases.len()
    ))
    .id_salt(("plugin-tool-aliases", scope, plugin_id, &tool.id))
    .default_open(cfg!(feature = "ui-preview"))
    .show(ui, |ui| {
        ui.weak("由插件清单声明；只用于搜索，不改变权限或执行行为。");
        for alias in &tool.aliases {
            ui.label(alias);
        }
    });
}
struct DiscoveryJob {
    id: String,
    source: String,
    settings_key: String,
    cancel: Arc<AtomicBool>,
    receiver: Receiver<Result<crate::model_discovery::Report, String>>,
}
struct UpdateCheckJob {
    source: String,
    cancel: Arc<AtomicBool>,
    receiver: Receiver<Result<plugins::AuthenticatedManifest, String>>,
}
struct CatalogCheckJob {
    source: String,
    source_id: Option<String>,
    cancel: Arc<AtomicBool>,
    receiver: Receiver<Result<plugins::AuthenticatedCatalog, String>>,
}
pub struct PluginState {
    pub store: Store,
    settings: crate::plugin_settings::Settings,
    library: crate::chat_library::Library,
    prompt_library: crate::prompt_library::Library,
    pub selected: Option<String>,
    path: String,
    preview: Option<Vec<u8>>,
    preview_origin: Option<String>,
    update_url: String,
    catalog_url: String,
    catalog_source_name: String,
    catalog_search: String,
    catalog: Option<plugins::AuthenticatedCatalog>,
    catalog_diff: Option<plugins::CatalogDiff>,
    catalog_running: Option<CatalogCheckJob>,
    publisher_name_input: String,
    publisher_key_input: String,
    update_running: Option<UpdateCheckJob>,
    message: String,
    drafts: HashMap<String, Draft>,
    token: String,
    use_saved: bool,
    discovery_token: String,
    discovery_running: Option<DiscoveryJob>,
    credential_message: String,
    pending_remove: Option<String>,
    pending_rollback: Option<String>,
    copied_at: Option<std::time::Instant>,
    running: Option<(String, Receiver<Result<String, String>>)>,
    stream: Option<StreamControl>,
    request_cancel: Option<Arc<AtomicBool>>,
    execution_job: Job,
    execution_instance: Option<String>,
    discovery_job: Job,
    discovery_instance: Option<String>,
    update_job: Job,
    update_instance: Option<String>,
    catalog_job: Job,
    catalog_instance: Option<String>,
    #[cfg(feature = "ui-preview")]
    preview_stream_sender: Option<mpsc::Sender<Result<String, String>>>,
}
impl PluginState {
    #[cfg(feature = "ui-preview")]
    pub fn preview_update(&mut self) {
        use ring::signature::KeyPair;
        const CURRENT: &[u8] = include_bytes!("../plugins-examples/local-text.json");
        if !self
            .store
            .packages
            .iter()
            .any(|p| p.manifest.id == "local-text")
        {
            self.store.install(CURRENT).unwrap();
        }
        self.store.set_enabled("local-text", true).unwrap();
        let pkcs8 =
            ring::signature::Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new())
                .unwrap();
        let publisher_key = ring::signature::Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
        let publisher_name = "示例签名发布者（预览）";
        let publisher_public_key = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            publisher_key.public_key().as_ref(),
        );
        self.store
            .pin_publisher(publisher_name, &publisher_public_key)
            .unwrap();
        let mut candidate: serde_json::Value = serde_json::from_slice(CURRENT).unwrap();
        candidate["version"] = serde_json::Value::String("1.1.0".into());
        candidate["tools"][0]["version"] = serde_json::Value::String("0.2.0".into());
        candidate["tools"][0]["version_history"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "version":"0.2.0",
                "date":"2026-10-14",
                "changes":"新增 HTTP 格式化适配器"
            }));
        candidate["tools"][0]["name"] = serde_json::Value::String("保持顺序去重（新版）".into());
        candidate["tools"][0]["adapter"] = serde_json::json!({
            "type":"http",
            "url":"http://127.0.0.1:11434/api/generate",
            "method":"POST",
            "body":{"prompt":"$input"}
        });
        self.path = "local-text-v1.1.0-preview.json".into();
        self.update_url = "https://example.com/plugins/local-text.json".into();
        let candidate = serde_json::to_vec_pretty(&candidate).unwrap();
        let signed = plugins::sign_manifest_with_pkcs8(&candidate, pkcs8.as_ref()).unwrap();
        let verified =
            plugins::verify_signed_manifest(&signed, &self.store.trusted_publishers).unwrap();
        self.preview = Some(verified.manifest);
        self.preview_origin = Some(format!(
            "示例预览 · 签名已验证 · {} · SHA-256 {}",
            verified.publisher_name, verified.publisher_id
        ));
        self.message = "示例差异预览 · 未执行升级".into();
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_catalog(&mut self) {
        let directory_signer = self
            .store
            .trusted_publishers
            .iter()
            .find(|publisher| publisher.name == "示例目录签名者（预览）")
            .cloned()
            .unwrap_or_else(|| {
                plugins::generate_publisher_keypair("示例目录签名者（预览）")
                    .unwrap()
                    .1
            });
        let trusted_author = self
            .store
            .trusted_publishers
            .iter()
            .find(|publisher| publisher.name == "示例插件作者（已固定）")
            .cloned()
            .unwrap_or_else(|| {
                plugins::generate_publisher_keypair("示例插件作者（已固定）")
                    .unwrap()
                    .1
            });
        let untrusted_author = self
            .store
            .trusted_publishers
            .iter()
            .find(|publisher| publisher.name == "示例插件作者（未固定）")
            .cloned()
            .unwrap_or_else(|| {
                plugins::generate_publisher_keypair("示例插件作者（未固定）")
                    .unwrap()
                    .1
            });
        self.store
            .pin_publisher(&directory_signer.name, &directory_signer.public_key)
            .unwrap();
        self.store
            .pin_publisher(&trusted_author.name, &trusted_author.public_key)
            .unwrap();
        self.catalog_url = "https://plugins.example.test/catalog-signed.json".into();
        let source_id = self
            .store
            .catalog_sources
            .iter()
            .find(|source| source.url == self.catalog_url)
            .map(|source| source.id.clone())
            .unwrap_or_else(|| {
                self.store
                    .add_catalog_source("示例团队目录", &self.catalog_url)
                    .unwrap()
            });
        self.store
            .bind_catalog_source_publisher(&source_id, &directory_signer.id)
            .unwrap();
        let previous = plugins::AuthenticatedCatalog {
            catalog: plugins::PluginCatalog {
                schema_version: 2,
                entries: vec![plugins::CatalogEntry {
                    plugin_id: "local-text".into(),
                    name: "本地文本工具集".into(),
                    version: "1.1.0".into(),
                    description: "文本整理、编码转换与结构化数据处理".into(),
                    manifest_url: "https://plugins.example.test/local-text-1.1.0.json".into(),
                    publisher_id: trusted_author.id.clone(),
                    host_app_version_req: Some(">=0.82.0-dev.1, <0.83.0".into()),
                    plugin_manifest_schema_version: Some(1),
                }],
            },
            publisher_id: directory_signer.id.clone(),
            publisher_name: directory_signer.name.clone(),
        };
        self.store
            .record_verified_catalog(&source_id, &previous)
            .unwrap();
        self.catalog_search.clear();
        let current = plugins::AuthenticatedCatalog {
            publisher_id: directory_signer.id,
            publisher_name: directory_signer.name,
            catalog: plugins::PluginCatalog {
                schema_version: 2,
                entries: vec![
                    plugins::CatalogEntry {
                        plugin_id: "local-text".into(),
                        name: "本地文本工具集（更新示例）".into(),
                        version: "1.2.0".into(),
                        description: "文本整理与结构化数据处理，新增字段级差异".into(),
                        manifest_url: "https://cdn.example.test/local-text-1.2.0.json".into(),
                        publisher_id: trusted_author.id,
                        host_app_version_req: Some(">=0.82.0-dev.105, <0.83.0".into()),
                        plugin_manifest_schema_version: Some(1),
                    },
                    plugins::CatalogEntry {
                        plugin_id: "doc-helper".into(),
                        name: "文档批处理助手".into(),
                        version: "0.8.1".into(),
                        description: "批量提取、重命名并生成文档目录".into(),
                        manifest_url: "https://plugins.example.test/doc-helper-0.8.1.json".into(),
                        publisher_id: untrusted_author.id,
                        host_app_version_req: Some(">=0.81.0, <0.82.0".into()),
                        plugin_manifest_schema_version: Some(1),
                    },
                ],
            },
        };
        self.catalog_diff = Some(
            self.store
                .record_verified_catalog(&source_id, &current)
                .unwrap(),
        );
        self.catalog = Some(current);
        self.message = "示例目录 · 展示目录验签与逐插件作者信任状态".into();
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_alias_manifest(&mut self) {
        self.preview = Some(include_bytes!("../plugins-examples/local-text.json").to_vec());
        self.preview_origin = Some("本机合成的中英文别名清单预览".into());
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_prompts(&mut self) {
        self.preview_conversation();
        let draft = self.drafts.get_mut("plugin:openai-local/chat").unwrap();
        draft.prompts.preview_fixture("plugin:openai-local/chat");
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_prompt_editor(&mut self) {
        self.preview_prompts();
        let draft = self.drafts.get_mut("plugin:openai-local/chat").unwrap();
        draft.prompts.preview_editor();
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_library(&mut self) {
        self.preview_conversation();
        let draft = self.drafts.get_mut("plugin:openai-local/chat").unwrap();
        draft.library_name = "示例项目讨论".into();
        draft.library_entries = vec![crate::chat_library::Entry {
            id: "preview-only".into(),
            name: draft.library_name.clone(),
            saved_at: "示例预览".into(),
            turns: draft.conversation.turns().len(),
            bytes: draft.conversation.bytes(),
        }];
        draft.library_skipped = 0;
        draft.library_truncated = false;
        draft.library_loaded = true;
        draft.library_open = true;
        draft.library_selected = Some("preview-only".into());
        draft.library_preview = Some(
            crate::conversation::Conversation::import(
                &draft.conversation.export(true).unwrap(),
                draft.conversation.binding().into(),
            )
            .unwrap(),
        );
        draft.chat_message = "示例会话库 · 截图未发送请求".into();
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_attachments(&mut self) {
        self.preview_conversation();
        let draft = self.drafts.get_mut("plugin:openai-local/chat").unwrap();
        draft.conversation.clear();
        draft.input = "检查附件中的配置，并说明 enabled 字段。".into();
        draft.attachments.preview();
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_import(&mut self) {
        self.preview_conversation();
        let draft = self.drafts.get_mut("plugin:openai-local/chat").unwrap();
        let mut candidate = crate::conversation::Conversation::default();
        candidate.bind(draft.conversation.binding().into());
        candidate
            .complete(
                "如何校验 JSON？".into(),
                "将文本放入 JSON 工具并执行校验。".into(),
            )
            .unwrap();
        draft.import_preview = Some(candidate);
        draft.import_open = true;
        draft.import_path = "示例会话.json（预览数据）".into();
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_stream(&mut self) {
        self.preview_conversation();
        let draft = self.drafts.get_mut("plugin:openai-local/chat").unwrap();
        draft.conversation.clear();
        draft.stream_mode = true;
        draft.output_completed = false;
        draft.chat_message = "示例流式状态 · 截图未发送请求".into();
        let (sender, receiver) = mpsc::channel();
        self.running = Some(("plugin:openai-local/chat".into(), receiver));
        self.preview_stream_sender = Some(sender);
        self.stream = Some(StreamControl {
            cancel: Arc::new(AtomicBool::new(false)),
            partial: Arc::new(Mutex::new(
                "JSON 使用明确的键值与数组结构；YAML 使用缩进表达层级。下面是同一配置的两种写法…"
                    .into(),
            )),
        });
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_conversation(&mut self) {
        self.preview_profiles();
        let id = "plugin:openai-local/chat";
        let tool = self
            .store
            .tool_refs()
            .find(|(key, _)| key == id)
            .unwrap()
            .1
            .clone();
        let draft = self.drafts.get_mut(id).unwrap();
        draft.profile_editor = false;
        draft.chat_mode = true;
        draft.conversation.bind(format!(
            "{}:{}",
            crate::plugin_settings::Settings::key(id, &tool),
            draft.model
        ));
        draft
            .conversation
            .complete(
                "JSON 与 YAML 有什么区别？".into(),
                "JSON 结构严格，适合程序交换数据；YAML 更方便手写配置，但需要注意缩进。".into(),
            )
            .unwrap();
        draft.input = "为刚才的区别补充一个配置示例。".into();
        draft.chat_message = "示例会话 · 截图未发送请求".into();
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_model_discovery(&mut self) {
        self.preview_profiles();
        let draft = self.drafts.get_mut("plugin:openai-local/chat").unwrap();
        draft.profile_editor = false;
        draft.discovery_open = true;
        draft.models_source = draft.endpoint.clone();
        draft.models = vec!["local-example-model".into(), "local-code-model".into()];
        draft.discovery_message = "示例结果 · 2 个模型（截图未发送请求）".into();
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_profiles(&mut self) {
        let id = "plugin:openai-local/chat";
        let tool = self
            .store
            .tool_refs()
            .find(|(key, _)| key == id)
            .unwrap()
            .1
            .clone();
        let connection = crate::plugin_settings::Connection {
            endpoint: "http://127.0.0.1:1234/v1/chat/completions".into(),
            model: "local-example-model".into(),
        };
        let existing = self
            .settings
            .profiles()
            .find(|(_, p)| p.name == "本机推理 · 示例")
            .map(|(id, _)| id.clone());
        let profile_id = match existing {
            Some(id) => id,
            None => self
                .settings
                .save_profile(
                    None,
                    crate::plugin_settings::Profile {
                        name: "本机推理 · 示例".into(),
                        shape: crate::plugin_settings::request_shape(&tool).unwrap(),
                        connection: connection.clone(),
                    },
                )
                .unwrap(),
        };
        self.drafts.insert(
            id.into(),
            Draft {
                endpoint: connection.endpoint,
                model: connection.model,
                settings_key: crate::plugin_settings::Settings::key(id, &tool),
                profile_id: Some(profile_id),
                profile_name: "本机推理 · 示例".into(),
                profile_editor: true,
                ..Default::default()
            },
        );
    }
    pub fn new(root: PathBuf) -> Self {
        Self {
            library: crate::chat_library::Library::new(root.join("sessions")),
            prompt_library: crate::prompt_library::Library::new(root.join("prompts")),
            settings: crate::plugin_settings::Settings::load(
                root.join("settings/connections.json"),
            ),
            store: Store::load(root),
            selected: None,
            path: String::new(),
            preview: None,
            preview_origin: None,
            update_url: String::new(),
            catalog_url: String::new(),
            catalog_source_name: String::new(),
            catalog_search: String::new(),
            catalog: None,
            catalog_diff: None,
            catalog_running: None,
            publisher_name_input: String::new(),
            publisher_key_input: String::new(),
            update_running: None,
            message: String::new(),
            drafts: HashMap::new(),
            token: String::new(),
            use_saved: false,
            discovery_token: String::new(),
            discovery_running: None,
            credential_message: String::new(),
            pending_remove: None,
            pending_rollback: None,
            copied_at: None,
            running: None,
            stream: None,
            request_cancel: None,
            execution_job: Job::default(),
            execution_instance: None,
            discovery_job: Job::default(),
            discovery_instance: None,
            update_job: Job::default(),
            update_instance: None,
            catalog_job: Job::default(),
            catalog_instance: None,
            #[cfg(feature = "ui-preview")]
            preview_stream_sender: None,
        }
    }
    pub fn select(&mut self, id: &str) {
        if self.selected.as_deref() != Some(id) {
            self.clear_token();
        }
        self.selected = Some(id.into());
    }
    pub fn clear_token(&mut self) {
        self.use_saved = false;
        self.credential_message.clear();
        self.token.clear();
        self.discovery_token.clear();
    }
    fn check_update_source(&mut self) -> anyhow::Result<()> {
        self.begin_update_source(None)
    }
    fn check_catalog_entry(&mut self, entry: plugins::CatalogEntry) -> anyhow::Result<()> {
        if let plugins::CatalogCompatibility::Incompatible(reason) = plugins::catalog_compatibility(
            &entry,
            &plugins::current_host_version(),
            plugins::PLUGIN_MANIFEST_SCHEMA_VERSION,
        ) {
            anyhow::bail!("插件目录条目与当前应用不兼容：{reason}");
        }
        self.update_url = entry.manifest_url.clone();
        self.begin_update_source(Some(entry))
    }
    fn begin_update_source(
        &mut self,
        expected: Option<plugins::CatalogEntry>,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.update_running.is_none() && self.catalog_running.is_none(),
            "插件目录或清单检查正在运行"
        );
        let source = self.update_url.trim().to_owned();
        anyhow::ensure!(!source.is_empty(), "请输入 HTTPS 插件清单地址");
        anyhow::ensure!(
            !self.store.trusted_publishers.is_empty(),
            "请先从独立可信渠道核对并固定插件发布者公钥"
        );
        plugins::endpoint(&source)?;
        self.preview = None;
        self.preview_origin = None;
        self.message.clear();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let (sender, receiver) = mpsc::channel();
        let trusted_publishers = self.store.trusted_publishers.clone();
        self.update_job.begin();
        self.update_instance = Some(uuid::Uuid::new_v4().to_string());
        self.update_running = Some(UpdateCheckJob {
            source: source.clone(),
            cancel,
            receiver,
        });
        std::thread::spawn(move || {
            let result =
                plugins::fetch_manifest_cancellable(&source, &trusted_publishers, &worker_cancel)
                    .and_then(|verified| {
                        if let Some(expected) = expected {
                            let manifest = plugins::parse(&verified.manifest)?;
                            anyhow::ensure!(
                                manifest.id == expected.plugin_id
                                    && manifest.version == expected.version
                                    && verified.publisher_id == expected.publisher_id,
                                "取得的插件清单与已签名目录中的 ID、版本或发布者不一致"
                            );
                            anyhow::ensure!(
                                expected
                                    .plugin_manifest_schema_version
                                    .is_none_or(|schema| manifest.schema_version == schema),
                                "取得的插件清单 schema 与已签名目录声明不一致"
                            );
                        }
                        Ok(verified)
                    })
                    .map_err(|error| format!("{error:#}"));
            let _ = sender.send(result);
        });
        Ok(())
    }
    fn check_catalog_source(&mut self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.catalog_running.is_none() && self.update_running.is_none(),
            "插件目录或清单检查正在运行"
        );
        let entered_source = self.catalog_url.trim();
        anyhow::ensure!(
            !entered_source.is_empty(),
            "请输入已签名插件目录的 HTTPS 地址"
        );
        let source = plugins::endpoint(entered_source)?.to_string();
        anyhow::ensure!(
            !self.store.trusted_publishers.is_empty(),
            "请先从独立可信渠道核对并固定插件目录发布者公钥"
        );
        self.catalog_url = source.clone();
        let saved_source = self
            .store
            .catalog_sources
            .iter()
            .find(|saved| saved.url == source)
            .cloned();
        self.catalog = None;
        self.catalog_diff = None;
        self.message.clear();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let trusted_publishers = self.store.trusted_publishers.clone();
        let source_id = saved_source.as_ref().map(|saved| saved.id.clone());
        let expected_publisher_id = saved_source
            .as_ref()
            .and_then(|saved| saved.publisher_id.clone());
        let worker_expected_publisher_id = expected_publisher_id.clone();
        let (sender, receiver) = mpsc::channel();
        self.catalog_job.begin();
        self.catalog_instance = Some(uuid::Uuid::new_v4().to_string());
        self.catalog_running = Some(CatalogCheckJob {
            source: source.clone(),
            source_id,
            cancel,
            receiver,
        });
        std::thread::spawn(move || {
            let result =
                plugins::fetch_catalog_cancellable(&source, &trusted_publishers, &worker_cancel)
                    .and_then(|catalog| {
                        anyhow::ensure!(
                            worker_expected_publisher_id
                                .as_deref()
                                .is_none_or(|expected| expected == catalog.publisher_id),
                            "目录来源签名者与首次验证时已固定的签名者不一致"
                        );
                        Ok(catalog)
                    })
                    .map_err(|error| format!("{error:#}"));
            let _ = sender.send(result);
        });
        Ok(())
    }
    pub fn is_running(&self) -> bool {
        self.running.is_some()
            || self.discovery_running.is_some()
            || self.update_running.is_some()
            || self.catalog_running.is_some()
    }
    pub(crate) fn task_snapshots(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        if let (Some(id), Some(mut row)) = (
            self.catalog_instance.as_ref(),
            self.catalog_job.snapshot(
                "plugin-catalog-check",
                "签名插件目录检查",
                self.catalog_running.is_some(),
            ),
        ) {
            row.instance = Some(id.clone());
            rows.push(row);
        }
        if let (Some(id), Some(mut row)) = (
            self.execution_instance.as_ref(),
            self.execution_job.snapshot(
                "plugin-execution",
                "插件工具请求",
                self.stream.is_some() || self.request_cancel.is_some(),
            ),
        ) {
            row.instance = Some(id.clone());
            rows.push(row);
        }
        if let (Some(id), Some(mut row)) = (
            self.discovery_instance.as_ref(),
            self.discovery_job.snapshot(
                "plugin-model-discovery",
                "插件模型列表查询",
                self.discovery_running.is_some(),
            ),
        ) {
            row.instance = Some(id.clone());
            rows.push(row);
        }
        if let (Some(id), Some(mut row)) = (
            self.update_instance.as_ref(),
            self.update_job.snapshot(
                "plugin-update-check",
                "插件更新清单检查",
                self.update_running.is_some(),
            ),
        ) {
            row.instance = Some(id.clone());
            rows.push(row);
        }
        rows
    }

    pub(crate) fn handoff_result(&self) -> Option<(String, String)> {
        if self.running.is_some() {
            return None;
        }
        let selected = self.selected.as_ref()?;
        let draft = self.drafts.get(selected)?;
        if !draft.output_completed || !draft.output_handoffable || draft.output.trim().is_empty() {
            return None;
        }
        Some(("插件最终结果".into(), draft.output.clone()))
    }
    pub(crate) fn cancel_task(&mut self, id: &str, generation: u64) -> bool {
        let catalog_generation = self
            .catalog_job
            .snapshot("plugin-catalog-check", "签名插件目录检查", true)
            .is_some_and(|row| {
                self.catalog_instance.as_deref() == Some(id)
                    && row.generation == generation
                    && row.phase.active()
            });
        if let Some(job) = self.catalog_running.as_ref()
            && self.catalog_instance.as_deref() == Some(id)
            && catalog_generation
        {
            job.cancel.store(true, Ordering::Relaxed);
            self.catalog_job.cancelling();
            return true;
        }
        let update_generation = self
            .update_job
            .snapshot("plugin-update-check", "插件更新清单检查", true)
            .is_some_and(|row| {
                self.update_instance.as_deref() == Some(id)
                    && row.generation == generation
                    && row.phase.active()
            });
        if let Some(job) = self.update_running.as_ref()
            && self.update_instance.as_deref() == Some(id)
            && update_generation
        {
            job.cancel.store(true, Ordering::Relaxed);
            self.update_job.cancelling();
            return true;
        }
        let discovery_generation = self
            .discovery_job
            .snapshot("plugin-model-discovery", "插件模型列表查询", true)
            .is_some_and(|row| {
                self.discovery_instance.as_deref() == Some(id)
                    && row.generation == generation
                    && row.phase.active()
            });
        if let Some(job) = self.discovery_running.as_ref()
            && job.id == id
            && discovery_generation
        {
            job.cancel.store(true, Ordering::Relaxed);
            self.discovery_job.cancelling();
            return true;
        }
        let matches_running = self
            .running
            .as_ref()
            .is_some_and(|(active, _)| active == id);
        let matches_generation = self
            .execution_job
            .snapshot("plugin-execution", "插件工具请求", true)
            .is_some_and(|row| {
                self.execution_instance.as_deref() == Some(id)
                    && row.generation == generation
                    && row.phase.active()
            });
        if !matches_running || !matches_generation {
            return false;
        }
        if let Some(stream) = self.stream.as_ref() {
            stream.cancel.store(true, Ordering::Relaxed);
        } else if let Some(cancel) = self.request_cancel.as_ref() {
            cancel.store(true, Ordering::Relaxed);
        } else {
            return false;
        }
        self.execution_job.cancelling();
        true
    }
    pub fn poll(&mut self) -> Option<String> {
        if let Some(job) = &self.catalog_running {
            let source_id = job.source_id.clone();
            let result = match job.receiver.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Disconnected) => Some(Err("插件目录检查线程已退出".into())),
                Err(mpsc::TryRecvError::Empty) => None,
            };
            if let Some(result) = result {
                let cancelled = job.cancel.load(Ordering::Relaxed);
                let succeeded = result.is_ok() && !cancelled;
                if self.catalog_url.trim() == job.source {
                    if cancelled {
                        self.catalog = None;
                        self.message = "插件目录检查已取消".into();
                    } else {
                        match result {
                            Ok(verified) => {
                                let count = verified.catalog.entries.len();
                                let (diff, persistence_error) = match source_id.as_deref() {
                                    Some(id) => {
                                        match self.store.record_verified_catalog(id, &verified) {
                                            Ok(diff) => (Some(diff), None),
                                            Err(error) => (None, Some(error.to_string())),
                                        }
                                    }
                                    None => (None, None),
                                };
                                self.catalog_diff = diff;
                                self.catalog = Some(verified);
                                self.message = if let Some(error) = persistence_error {
                                    format!(
                                        "插件目录签名验证通过，共 {count} 项；本机变更记录保存失败：{error}"
                                    )
                                } else if self
                                    .catalog_diff
                                    .as_ref()
                                    .is_some_and(|diff| !diff.had_previous_check)
                                {
                                    format!(
                                        "插件目录签名验证通过，共 {count} 项；已建立本机变更基线"
                                    )
                                } else {
                                    format!(
                                        "插件目录签名验证通过，共 {count} 项；获取插件仍需通过发布者固定和独立清单验签"
                                    )
                                };
                            }
                            Err(error) => {
                                self.catalog = None;
                                self.message = error;
                            }
                        }
                    }
                } else {
                    self.catalog = None;
                    self.message = "目录地址在请求期间发生变化，已丢弃旧响应".into();
                }
                self.catalog_job.finish(
                    if cancelled {
                        Phase::Cancelled
                    } else if succeeded {
                        Phase::Done
                    } else {
                        Phase::Failed
                    },
                    if cancelled {
                        "插件目录检查已取消；未安装任何内容"
                    } else if succeeded {
                        "签名插件目录已验证；尚未安装或启用"
                    } else {
                        "插件目录检查失败；详情留在插件管理页"
                    },
                );
                self.catalog_running = None;
                return Some("插件目录检查已结束".into());
            }
        }
        if let Some(job) = &self.update_running {
            let result = match job.receiver.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Disconnected) => Some(Err("更新清单检查线程已退出".into())),
                Err(mpsc::TryRecvError::Empty) => None,
            };
            if let Some(result) = result {
                let cancelled = job.cancel.load(Ordering::Relaxed);
                let succeeded = result.is_ok() && !cancelled;
                if self.update_url.trim() == job.source {
                    if cancelled {
                        self.message = "更新清单检查已取消".into();
                    } else {
                        match result {
                            Ok(verified) => {
                                self.preview = Some(verified.manifest);
                                self.preview_origin = Some(format!(
                                    "用户指定的 HTTPS 更新源 · 签名已验证 · {} · SHA-256 {}",
                                    verified.publisher_name, verified.publisher_id
                                ));
                                self.path.clear();
                                self.message =
                                    "发布者签名验证通过；请继续核对插件 ID、版本和权限差异，再手动确认升级".into();
                            }
                            Err(error) => {
                                self.preview = None;
                                self.preview_origin = None;
                                self.message = error;
                            }
                        }
                    }
                } else {
                    self.message = "更新源在请求期间发生变化，已丢弃旧响应".into();
                }
                self.update_job.finish(
                    if cancelled {
                        Phase::Cancelled
                    } else if succeeded {
                        Phase::Done
                    } else {
                        Phase::Failed
                    },
                    if cancelled {
                        "更新清单检查已取消；未安装任何内容"
                    } else if succeeded {
                        "更新清单已取得；尚未安装或启用"
                    } else {
                        "更新清单检查失败；详情留在插件管理页"
                    },
                );
                self.update_running = None;
                return Some("插件更新清单检查已结束".into());
            }
        }
        if let Some(job) = &self.discovery_running {
            let result = match job.receiver.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Disconnected) => Some(Err("模型查询线程已退出".into())),
                Err(mpsc::TryRecvError::Empty) => None,
            };
            if let Some(result) = result {
                let cancelled = job.cancel.load(Ordering::Relaxed);
                let succeeded = result.is_ok() && !cancelled;
                let draft = self.drafts.entry(job.id.clone()).or_default();
                if draft.endpoint == job.source && draft.settings_key == job.settings_key {
                    draft.models.clear();
                    draft.models_source = job.source.clone();
                    draft.discovery_message = if cancelled {
                        "模型列表查询已取消".into()
                    } else {
                        match result {
                            Ok(report) => {
                                draft.models = report.models;
                                format!(
                                    "模型列表查询成功 · {} ms · {} 个模型；不代表生成请求一定可用",
                                    report.elapsed_ms,
                                    draft.models.len()
                                )
                            }
                            Err(error) => format!("查询失败：{error}"),
                        }
                    };
                }
                self.discovery_job.finish(
                    if cancelled {
                        Phase::Cancelled
                    } else if succeeded {
                        Phase::Done
                    } else {
                        Phase::Failed
                    },
                    if cancelled {
                        "模型列表查询已取消；结果留在插件连接设置中"
                    } else if succeeded {
                        "模型列表查询完成；结果留在插件连接设置中"
                    } else {
                        "模型列表查询失败；详情留在插件连接设置中"
                    },
                );
                self.discovery_running = None;
                return Some("模型查询已结束，请查看原工具的连接检查结果".into());
            }
        }
        let (id, rx) = self.running.as_ref()?;
        if let Some(stream) = &self.stream {
            if let Ok(partial) = stream.partial.lock() {
                self.drafts.entry(id.clone()).or_default().output = partial.clone();
            }
        }
        let mut result = match rx.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Disconnected) => Err("执行线程已退出".into()),
            Err(mpsc::TryRecvError::Empty) => return None,
        };
        let stream_cancelled = self
            .stream
            .as_ref()
            .is_some_and(|stream| stream.cancel.load(Ordering::Relaxed));
        let request_cancelled = self
            .request_cancel
            .as_ref()
            .is_some_and(|cancel| cancel.load(Ordering::Relaxed));
        let cancelled = stream_cancelled || request_cancelled;
        if request_cancelled {
            result = Err("客户端请求已取消；服务端可能仍在处理".into());
        }
        let partial = self.stream.take().map(|stream| {
            if stream.cancel.load(Ordering::Relaxed) {
                result = Err("已停止，部分输出未加入上下文".into());
            }
            stream.partial.lock().map(|s| s.clone()).unwrap_or_default()
        });
        let succeeded = result.is_ok();
        let draft = self.drafts.entry(id.clone()).or_default();
        let probe_mode = std::mem::take(&mut draft.probe_mode);
        if let Some(previous) = draft.probe_previous_input.take() {
            draft.input = previous;
        }
        if let Some((binding, input)) = draft.pending_chat.take() {
            if draft.conversation.binding() == binding {
                if let Ok(output) = &result {
                    match draft.conversation.complete(input, output.clone()) {
                        Ok(()) => {
                            draft.input.clear();
                            draft.attachments.clear();
                            draft.chat_message.clear();
                        }
                        Err(error) => draft.chat_message = error.to_string(),
                    }
                } else {
                    draft.chat_message = "请求失败，消息与此前上下文保留；可再次运行重试".into();
                }
            }
        }
        draft.output_completed = succeeded;
        draft.output_handoffable = succeeded && !probe_mode;
        if probe_mode {
            draft.chat_message = if succeeded {
                "模型生成测试成功；这次输出没有加入会话或工具接力".into()
            } else if cancelled {
                "模型生成测试已取消；这次输出没有加入会话或工具接力".into()
            } else {
                "模型生成测试失败；检查接口、模型和认证设置".into()
            };
        }
        draft.output = result.unwrap_or_else(|e| match partial {
            Some(partial) if !partial.is_empty() => format!("{partial}\n\n[未完成] {e}"),
            _ => format!("失败：{e}"),
        });
        self.execution_job.finish(
            if cancelled {
                Phase::Cancelled
            } else if succeeded {
                Phase::Done
            } else {
                Phase::Failed
            },
            if stream_cancelled {
                "生成已停止；部分内容留在插件工作台"
            } else if request_cancelled {
                "客户端请求已取消；服务端可能仍在处理"
            } else if succeeded {
                "插件请求完成；结果留在插件工作台"
            } else {
                "插件请求失败；详情留在插件工作台"
            },
        );
        self.request_cancel = None;
        self.running = None;
        Some(
            if succeeded {
                "插件执行完成"
            } else {
                "插件执行失败，请查看结果"
            }
            .into(),
        )
    }
    pub fn ui(&mut self, ui: &mut egui::Ui, allow_shortcuts: bool) {
        ui.heading(egui::RichText::new("插件与连接器").size(28.0));
        ui.label(
            "安装工具定义，按需连接本机或远程服务。已启用工具会自动进入分类、收藏与 Ctrl K 搜索。",
        );
        ui.add_space(12.0);
        if self.selected.is_some() && ui.button("← 返回插件管理").clicked() {
            self.selected = None;
            self.clear_token();
        }
        if let Some(id) = self.selected.clone() {
            if let Some((_, mut tool)) = self
                .store
                .tool_refs()
                .find(|(key, _)| key == &id)
                .map(|(key, t)| (key, t.clone()))
            {
                let busy = self.running.as_ref().is_some_and(|(key, _)| key == &id)
                    || self
                        .discovery_running
                        .as_ref()
                        .is_some_and(|job| job.id == id);
                let get_only = matches!(&tool.adapter,Adapter::Http{method,..} if method=="GET");
                ui.heading(if self.drafts.get(&id).is_some_and(|d| d.chat_mode) {
                    "多轮模型对话"
                } else {
                    &tool.name
                });
                ui.label(&tool.description);
                let settings_key = crate::plugin_settings::Settings::key(&id, &tool);
                let profile_tool = tool.clone();
                let default_endpoint = match &tool.adapter {
                    Adapter::Http { url, .. } => url.clone(),
                    _ => String::new(),
                };
                let draft = self.drafts.entry(id.clone()).or_default();
                if draft.settings_key != settings_key {
                    let saved = self.settings.get(&settings_key);
                    draft.model = saved.map_or_else(|| tool.model.clone(), |v| v.model.clone());
                    draft.endpoint =
                        saved.map_or_else(|| default_endpoint.clone(), |v| v.endpoint.clone());
                    draft.settings_key = settings_key.clone();
                    draft.settings_message.clear();
                    draft.profile_id = None;
                    draft.profile_name.clear();
                    draft.profile_delete = false;
                    self.token.clear();
                    self.discovery_token.clear();
                    self.use_saved = false;
                    self.credential_message.clear();
                }
                let mut connection_valid = true;
                if let Adapter::Http {
                    url, method, body, ..
                } = &mut tool.adapter
                {
                    ui.add_enabled_ui(!busy, |ui| {
                        egui::CollapsingHeader::new("连接设置 · 地址与模型")
                            .id_salt(("connection-settings", &id))
                            .default_open(draft.profile_editor)
                            .show(ui, |ui| {
                                ui.label("完整接口地址（远程 HTTPS / 本机 HTTP）");
                                if ui
                                    .add(
                                        egui::TextEdit::singleline(&mut draft.endpoint)
                                            .desired_width(f32::INFINITY),
                                    )
                                    .changed()
                                {
                                    self.token.clear();
                                    self.discovery_token.clear();
                                    self.use_saved = false;
                                    self.credential_message.clear();
                                    draft.settings_message.clear();
                                }
                                if method == "POST" && plugins::uses_model(body) {
                                    ui.horizontal(|ui| {
                                        ui.label("模型名称");
                                        if ui
                                            .add(
                                                egui::TextEdit::singleline(&mut draft.model)
                                                    .hint_text("填入服务提供的模型名称"),
                                            )
                                            .changed()
                                        {
                                            draft.settings_message.clear();
                                        }
                                    });
                                }
                                if profiles_ui(ui, &mut self.settings, draft, &profile_tool) {
                                    self.token.clear();
                                    self.discovery_token.clear();
                                    self.use_saved = false;
                                    self.credential_message.clear();
                                }
                                let value = crate::plugin_settings::Connection {
                                    endpoint: draft.endpoint.clone(),
                                    model: draft.model.clone(),
                                };
                                let changed = self.settings.get(&settings_key) != Some(&value);
                                ui.horizontal_wrapped(|ui| {
                                    if ui
                                        .add_enabled(
                                            changed
                                                && value.validate().is_ok()
                                                && self.settings.error.is_none(),
                                            egui::Button::new("保存连接设置"),
                                        )
                                        .clicked()
                                    {
                                        draft.settings_message =
                                            match self.settings.set(&settings_key, Some(value)) {
                                                Ok(()) => "连接设置已保存；未保存令牌".into(),
                                                Err(_) => "保存失败，请检查配置目录写入权限".into(),
                                            };
                                    }
                                    if ui
                                        .add_enabled(
                                            self.settings.error.is_none(),
                                            egui::Button::new("恢复插件默认"),
                                        )
                                        .clicked()
                                    {
                                        match self.settings.set(&settings_key, None) {
                                            Ok(()) => {
                                                draft.endpoint = default_endpoint.clone();
                                                draft.model = tool.model.clone();
                                                self.token.clear();
                                                self.discovery_token.clear();
                                                self.use_saved = false;
                                                self.credential_message.clear();
                                                draft.settings_message =
                                                    "已恢复插件默认；系统凭据保留".into();
                                            }
                                            Err(_) => {
                                                draft.settings_message =
                                                    "恢复失败，原设置保留".into()
                                            }
                                        }
                                    }
                                    if changed {
                                        ui.weak("当前编辑可直接运行；保存后重启仍可使用");
                                    }
                                });
                                if !draft.settings_message.is_empty() {
                                    ui.label(&draft.settings_message);
                                }
                                if let Some(error) = &self.settings.error {
                                    ui.colored_label(ui.visuals().error_fg_color, error);
                                }
                            });
                    });
                    *url = draft.endpoint.clone();
                    if let Err(error) = (crate::plugin_settings::Connection {
                        endpoint: url.clone(),
                        model: draft.model.clone(),
                    })
                    .validate()
                    {
                        connection_valid = false;
                        ui.colored_label(ui.visuals().error_fg_color, error.to_string());
                    }
                    ui.label(format!("请求目标：{method} {url}"));
                    if method == "POST" && plugins::uses_model(body) {
                        ui.horizontal_wrapped(|ui| {
                            ui.label("当前模型");
                            ui.strong(if draft.model.trim().is_empty() {
                                "尚未设置 · 展开连接设置或发现模型"
                            } else {
                                &draft.model
                            });
                        });
                    }
                    ui.add_enabled_ui(!busy && connection_valid, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.checkbox(&mut self.use_saved, "使用已保存凭据");
                            ui.label("临时 Bearer 令牌（可选）");
                            ui.add_enabled(!self.use_saved, egui::TextEdit::singleline(&mut self.token).password(true));
                            if ui.small_button("清空输入").clicked() { self.token.clear(); }
                        });
                        egui::CollapsingHeader::new("保存与管理凭据")
                            .id_salt(("credential-management", &id)).show(ui, |ui| {
                            ui.horizontal_wrapped(|ui| {
                            if ui.add_enabled(!self.token.trim().is_empty() && !self.use_saved, egui::Button::new("保存 / 替换凭据")).clicked() {
                                self.credential_message = match crate::credentials::Target::plugin(&id, method, url).and_then(|target| crate::credentials::save(&target, &self.token)) {
                                    Ok(()) => { self.token.clear(); self.use_saved = true; "已保存到当前 Windows 用户的凭据管理器".into() },
                                    Err(e) => e.to_string(),
                                };
                            }
                            if ui.button("删除此工具凭据").clicked() {
                                self.credential_message = match crate::credentials::Target::plugin(&id, method, url).and_then(|target| crate::credentials::delete(&target)) {
                                    Ok(()) => { self.use_saved = false; "此工具当前地址的凭据已删除".into() },
                                    Err(e) => e.to_string(),
                                };
                            }
                            });
                            ui.label("凭据仅用于当前工具、请求方法和接口地址，保存在 Windows 凭据管理器。更换地址后需重新选择认证。");
                            ui.weak("删除不会中止已发送的请求。旧地址的凭据可在 Windows 凭据管理器中清理。");
                        });
                        if !self.credential_message.is_empty() { ui.label(&self.credential_message); }
                    });
                    if method == "POST" && plugins::uses_model(body) {
                        ui.toggle_value(&mut draft.discovery_open, "发现模型 / 检查列表连接");
                        if draft.discovery_open {
                            ui.group(|ui| {
                                let target = crate::model_discovery::target(url);
                                match &target {
                                    Ok(target) => { ui.label(format!("将发送 GET {target}")); }
                                    Err(error) => { ui.weak(error.to_string()); }
                                }
                                ui.label("仅查询模型列表，不发送对话或生成请求。查询认证需单独输入临时令牌；不会读取已保存的问答凭据。");
                                ui.horizontal_wrapped(|ui| {
                                    ui.label("列表查询令牌（可选）");
                                    ui.add_enabled(!busy, egui::TextEdit::singleline(&mut self.discovery_token).password(true));
                                    if ui.add_enabled(self.discovery_running.is_none() && self.running.is_none() && target.is_ok() && connection_valid, egui::Button::new("查询模型列表")).clicked() {
                                        let source = url.clone();
                                        let token = std::mem::take(&mut self.discovery_token);
                                        let cancel = Arc::new(AtomicBool::new(false));
                                        let worker_cancel = Arc::clone(&cancel);
                                        let (sender, receiver) = mpsc::channel();
                                        self.discovery_job.begin();
                                        self.discovery_instance = Some(id.clone());
                                        self.discovery_running = Some(DiscoveryJob { id: id.clone(), source: source.clone(), settings_key: settings_key.clone(), cancel, receiver });
                                        draft.models.clear();
                                        draft.models_source = source.clone();
                                        draft.discovery_message = "查询中，最长 10 秒；可切换页面或在任务中心取消".into();
                                        std::thread::spawn(move || {
                                            let result = crate::model_discovery::fetch_cancellable(&source, token, &worker_cancel).map_err(|e| e.to_string());
                                            let _ = sender.send(result);
                                        });
                                    }
                                });
                                let discovery_cancel = self
                                    .discovery_running
                                    .as_ref()
                                    .filter(|job| job.id == id)
                                    .map(|job| Arc::clone(&job.cancel));
                                if let Some(cancel) = discovery_cancel {
                                    let stopping = cancel.load(Ordering::Relaxed);
                                    ui.horizontal(|ui| {
                                        ui.spinner();
                                        ui.label(if stopping {
                                            "正在取消模型列表查询…"
                                        } else {
                                            "模型列表查询中 · 最长 10 秒"
                                        });
                                        if ui
                                            .add_enabled(!stopping, egui::Button::new("取消查询"))
                                            .clicked()
                                        {
                                            cancel.store(true, Ordering::Relaxed);
                                            self.discovery_job.cancelling();
                                        }
                                    });
                                }
                                if draft.models_source == *url {
                                    if !draft.discovery_message.is_empty() { ui.label(&draft.discovery_message); }
                                    if !draft.models.is_empty() {
                                        ui.add_enabled_ui(!busy, |ui| {
                                            egui::ComboBox::from_id_salt("discovered-models")
                                                .selected_text("选择模型并填入名称").width(280.0).show_ui(ui, |ui| {
                                                    for model in &draft.models {
                                                        if ui.selectable_label(draft.model == *model, model).clicked() {
                                                            draft.model = model.clone();
                                                            draft.settings_message.clear();
                                                        }
                                                    }
                                                });
                                        });
                                    }
                                }
                            });
                        }
                    }
                } else {
                    ui.label("本地配方 · 无网络请求");
                }
                let chat_supported = crate::conversation::supported(&tool);
                let model_probe_supported = match &tool.adapter {
                    Adapter::Http {
                        url, method, body, ..
                    } => {
                        method == "POST"
                            && crate::plugins::uses_model(body)
                            && crate::model_discovery::target(url).is_ok()
                    }
                    _ => false,
                };
                let can_probe =
                    chat_supported && model_probe_supported && !draft.model.trim().is_empty();
                let mut probe_clicked = false;
                if can_probe {
                    ui.horizontal_wrapped(|ui| {
                        probe_clicked = ui
                            .add_enabled(
                                !busy && connection_valid,
                                egui::Button::new("测试模型生成"),
                            )
                            .on_hover_text("仅发送固定提示“只回复 OK”，可能产生服务费用；结果不会进入会话或工具接力")
                            .clicked();
                        ui.weak("固定短提示，可能产生服务费用；不发送草稿、附件或历史会话，结果不接力");
                    });
                }
                let binding = format!(
                    "{}:{}",
                    crate::plugin_settings::Settings::key(&id, &tool),
                    draft.model
                );
                draft.bind_conversation(binding.clone());
                if chat_supported {
                    let prompt_action = ui
                        .add_enabled_ui(!busy, |ui| {
                            prompts::show(
                                ui,
                                &mut draft.prompts,
                                &self.prompt_library,
                                prompts::Current {
                                    tool_id: &id,
                                    input: &draft.input,
                                    model: &draft.model,
                                    stream: draft.stream_mode,
                                    multi_turn: draft.chat_mode,
                                },
                            )
                        })
                        .inner;
                    if let Some(action) = prompt_action {
                        draft.apply_prompt(
                            action,
                            &id,
                            &crate::plugin_settings::Settings::key(&id, &tool),
                        );
                    }
                    ui.add_enabled_ui(!busy, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            if ui.checkbox(&mut draft.chat_mode, "多轮上下文").changed()
                                && !draft.chat_mode
                            {
                                draft.conversation.clear();
                                draft.library_preview = None;
                                draft.chat_message = "已切回单轮并清空会话".into();
                            }
                            ui.checkbox(&mut draft.stream_mode, "流式输出（可停止）");
                            if draft.chat_mode {
                                ui.label(format!(
                                    "{} / 16 轮 · {} / 256 KiB",
                                    draft.conversation.turns().len(),
                                    draft.conversation.bytes().div_ceil(1024)
                                ));
                                if ui.button("清空会话").clicked() {
                                    draft.conversation.clear();
                                    draft.output.clear();
                                    draft.chat_message = "会话已清空".into();
                                }
                            }
                        });
                    });
                    if draft.chat_mode {
                        ui.weak("当前会话仅在内存中；只有主动保存才会写入本地会话库。发送时包含本工具此前成功的问答；切换模型或地址会清空上下文。");
                        ui.add_enabled_ui(!busy, |ui| {
                            egui::CollapsingHeader::new("本地会话库").default_open(draft.library_open).show(ui, |ui| {
                                ui.weak("主动保存成功问答到本机插件目录；正文可能包含你输入的信息。不会保存连接凭据、未发送草稿或未完成输出。");
                                if !draft.library_loaded {
                                    match self.library.list() {
                                        Ok(listing) => {
                                            draft.library_entries = listing.entries;
                                            draft.library_skipped = listing.skipped;
                                            draft.library_truncated = listing.truncated;
                                            draft.library_loaded = true;
                                        }
                                        Err(error) => draft.chat_message = format!("读取会话库失败：{error}"),
                                    }
                                }
                                ui.horizontal(|ui| {
                                    ui.add(egui::TextEdit::singleline(&mut draft.library_name).hint_text("会话名称，最多 80 字").desired_width(260.0));
                                    if ui.add_enabled(!draft.conversation.turns().is_empty(), egui::Button::new("保存当前会话")).clicked() {
                                        match self.library.save(&draft.library_name, &draft.conversation) {
                                            Ok(entry) => {
                                                draft.library_selected = Some(entry.id);
                                                draft.library_preview = None;
                                                draft.library_loaded = false;
                                                draft.chat_message = "会话已保存在本机；当前内存会话未改变".into();
                                            }
                                            Err(error) => draft.chat_message = format!("保存失败：{error}"),
                                        }
                                    }
                                });
                                if ui.button("刷新列表").clicked() { draft.library_loaded = false; }
                                if draft.library_skipped > 0 {
                                    ui.weak(format!("{} 个无效或超额文件未显示", draft.library_skipped));
                                }
                                if draft.library_truncated {
                                    ui.weak("会话文件超过扫描上限；请整理目录后再保存");
                                }
                                if draft.library_entries.is_empty() {
                                    ui.weak("尚无已保存会话");
                                } else {
                                    egui::ScrollArea::vertical().id_salt(("library-list", &id)).max_height(140.0).show(ui, |ui| {
                                        for entry in &draft.library_entries {
                                            let selected = draft.library_selected.as_deref() == Some(&entry.id);
                                            if ui.selectable_label(selected, format!("{} · {} 轮 · {}", entry.name, entry.turns, entry.saved_at)).clicked() {
                                                draft.library_selected = Some(entry.id.clone());
                                                draft.library_preview = None;
                                                draft.library_delete_confirm = false;
                                            }
                                        }
                                    });
                                    if let Some(selected) = draft.library_selected.clone() {
                                        ui.horizontal(|ui| {
                                            if ui.button("预览所选").clicked() {
                                                match self.library.load(&selected, binding.clone()) {
                                                    Ok((_, candidate)) => {
                                                        draft.library_preview = Some(candidate);
                                                        draft.chat_message.clear();
                                                    }
                                                    Err(error) => {
                                                        draft.library_preview = None;
                                                        draft.chat_message = format!("预览失败，当前会话保留：{error}");
                                                    }
                                                }
                                            }
                                            if ui.button("删除所选").clicked() { draft.library_delete_confirm = true; }
                                        });
                                        if draft.library_delete_confirm {
                                            ui.horizontal(|ui| {
                                                ui.weak("删除本机这份会话文件？其他导出副本不会删除。");
                                                if ui.button("确认删除").clicked() {
                                                    match self.library.delete(&selected) {
                                                        Ok(()) => {
                                                            draft.library_selected = None;
                                                            draft.library_preview = None;
                                                            draft.library_loaded = false;
                                                            draft.chat_message = "所选会话文件已删除；当前内存会话保留".into();
                                                        }
                                                        Err(error) => draft.chat_message = format!("删除失败：{error}"),
                                                    }
                                                    draft.library_delete_confirm = false;
                                                }
                                                if ui.button("取消").clicked() { draft.library_delete_confirm = false; }
                                            });
                                        }
                                    }
                                    if let Some(candidate) = &draft.library_preview {
                                        ui.label(format!("待恢复 {} 轮 · {} 字节；将替换当前 {} 轮", candidate.turns().len(), candidate.bytes(), draft.conversation.turns().len()));
                                        ui.horizontal(|ui| {
                                            if ui.button("确认恢复到当前会话").clicked() {
                                                draft.conversation = draft.library_preview.take().unwrap();
                                                draft.output.clear();
                                                draft.chat_message = "会话已恢复；输入草稿保留，尚未发送请求".into();
                                            }
                                            if ui.button("取消预览").clicked() { draft.library_preview = None; }
                                        });
                                        if let Some(candidate) = &draft.library_preview {
                                            egui::ScrollArea::vertical().id_salt(("library-preview", &id)).max_height(120.0).show(ui, |ui| {
                                                for (question, answer) in candidate.turns() {
                                                    ui.strong("你"); ui.label(question);
                                                    ui.strong("模型"); ui.label(answer); ui.separator();
                                                }
                                            });
                                        }
                                    }
                                }
                            });
                            egui::CollapsingHeader::new("导入会话 JSON").default_open(draft.import_open).show(ui, |ui| {
                                ui.weak("只读入问答。确认后替换当前历史；下一次发送会将这些内容交给当前模型，不会自动运行。");
                                if ui.add(egui::TextEdit::singleline(&mut draft.import_path).hint_text("会话 JSON 文件完整路径").desired_width(f32::INFINITY)).changed() {
                                    draft.import_preview = None;
                                }
                                if ui.button("读取并预览").clicked() {
                                    draft.import_preview = None;
                                    match crate::conversation::Conversation::import_file(&draft.import_path, binding.clone()) {
                                        Ok(candidate) => { draft.import_preview = Some(candidate); draft.chat_message.clear(); }
                                        Err(error) => draft.chat_message = format!("导入失败，当前会话保留：{error}"),
                                    }
                                }
                                if let Some(candidate) = &draft.import_preview {
                                    ui.label(format!("待导入 {} 轮 · {} 字节；将替换当前 {} 轮", candidate.turns().len(), candidate.bytes(), draft.conversation.turns().len()));
                                    egui::ScrollArea::vertical().id_salt(("import-preview", &id)).max_height(120.0).show(ui, |ui| {
                                        for (q, a) in candidate.turns() { ui.strong("你"); ui.label(q); ui.strong("模型"); ui.label(a); ui.separator(); }
                                    });
                                    ui.horizontal(|ui| {
                                        if ui.button("确认替换会话").clicked() {
                                            draft.conversation = draft.import_preview.take().unwrap();
                                            draft.output.clear();
                                            draft.chat_message = "会话已导入，输入草稿保留；尚未发送请求".into();
                                        }
                                        if ui.button("取消导入").clicked() { draft.import_preview = None; }
                                    });
                                }
                            });
                        });
                        if !draft.conversation.turns().is_empty() {
                            ui.collapsing("导出会话", |ui| {
                                ui.weak("仅导出成功问答；不包含连接设置、未发送输入或未完成输出。文件包含对话正文，请自行选择保存位置。");
                                ui.horizontal(|ui| {
                                    ui.selectable_value(&mut draft.export_json, false, "Markdown");
                                    ui.selectable_value(&mut draft.export_json, true, "JSON");
                                    if ui.button("复制会话").clicked() {
                                        match draft.conversation.export(draft.export_json) {
                                            Ok(text) => { ui.ctx().copy_text(text); draft.chat_message = "会话已复制".into(); }
                                            Err(error) => draft.chat_message = error.to_string(),
                                        }
                                    }
                                });
                                ui.add(egui::TextEdit::singleline(&mut draft.export_path).hint_text("新文件完整路径（.md 或 .json）").desired_width(f32::INFINITY));
                                if ui.add_enabled(!draft.export_path.trim().is_empty(), egui::Button::new("保存到新文件")).clicked() {
                                    draft.chat_message = match draft.conversation.export(draft.export_json)
                                        .and_then(|text| crate::workbench::save_new_file(&draft.export_path, &text)) {
                                            Ok(()) => "会话已保存；已有文件不会被覆盖".into(),
                                            Err(error) => error.to_string(),
                                        };
                                }
                            });
                            egui::ScrollArea::vertical()
                                .id_salt(("conversation", &id))
                                .max_height(180.0)
                                .show(ui, |ui| {
                                    for (index, (question, answer)) in
                                        draft.conversation.turns().iter().enumerate()
                                    {
                                        ui.strong(format!("你 · 第 {} 轮", index + 1));
                                        ui.label(question);
                                        ui.strong("模型");
                                        ui.label(answer);
                                        ui.separator();
                                    }
                                });
                        }
                    }
                    if !draft.chat_message.is_empty() {
                        ui.label(&draft.chat_message);
                    }
                } else {
                    draft.chat_mode = false;
                }
                ui.add_space(8.0);
                ui.separator();
                if !get_only {
                    ui.horizontal(|ui| {
                        ui.strong("输入");
                        if ui
                            .add_enabled(
                                !busy && draft.input.is_empty(),
                                egui::Button::new("填入示例"),
                            )
                            .clicked()
                        {
                            draft.input = tool.sample.clone();
                        }
                        if ui
                            .add_enabled(!busy, egui::Button::new("清空输入与结果"))
                            .clicked()
                        {
                            draft.input.clear();
                            draft.output.clear();
                            draft.attachments.clear();
                        }
                    });
                    ui.add_enabled_ui(!busy, |ui| {
                        ui.add_sized(
                            [
                                ui.available_width(),
                                if draft.chat_mode { 100.0 } else { 160.0 },
                            ],
                            egui::TextEdit::multiline(&mut draft.input)
                                .font(egui::TextStyle::Monospace),
                        );
                    });
                } else {
                    ui.label("此工具只读取接口结果，无需输入文本或模型名称。");
                }
                if chat_supported {
                    draft.attachments.ui(ui, !busy);
                }
                let shortcut = allow_shortcuts
                    && ui.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::Enter));
                let can_run =
                    self.running.is_none() && self.discovery_running.is_none() && connection_valid;
                let run_clicked = ui
                    .add_enabled(can_run, egui::Button::new("运行工具 · Ctrl Enter"))
                    .clicked();
                if probe_clicked {
                    draft.probe_previous_input =
                        Some(std::mem::replace(&mut draft.input, "只回复 OK".into()));
                    draft.probe_mode = true;
                    draft.chat_message = "已发起固定短提示测试；结果不会保存到会话".into();
                }
                if (run_clicked || shortcut || probe_clicked)
                    && self.running.is_none()
                    && self.discovery_running.is_none()
                    && connection_valid
                {
                    let probe_mode = draft.probe_mode;
                    let input = if probe_mode {
                        "只回复 OK".to_owned()
                    } else if chat_supported {
                        match draft.attachments.compose(&draft.input) {
                            Ok(input) => input,
                            Err(error) => {
                                draft.chat_message = error.to_string();
                                return;
                            }
                        }
                    } else {
                        draft.input.clone()
                    };
                    let mut messages = if draft.chat_mode && !probe_mode {
                        match draft.conversation.prepare(&input) {
                            Ok(messages) => Some(messages),
                            Err(error) => {
                                draft.chat_message = error.to_string();
                                return;
                            }
                        }
                    } else {
                        None
                    };
                    if !probe_mode && draft.attachments.has_images() {
                        let ollama = matches!(
                            &tool.adapter,
                            Adapter::Http {
                                response_pointer: Some(pointer),
                                ..
                            } if pointer == "/message/content"
                        );
                        let request_messages = messages.get_or_insert_with(|| {
                            vec![serde_json::json!({"role":"user","content":input.clone()})]
                        });
                        if let Err(error) =
                            draft
                                .attachments
                                .attach_to_messages(request_messages, &input, ollama)
                        {
                            draft.chat_message = error.to_string();
                            return;
                        }
                    }
                    let (input, model, token) = (
                        input,
                        draft.model.clone(),
                        if self.use_saved {
                            String::new()
                        } else {
                            self.token.clone()
                        },
                    );
                    let use_saved = self.use_saved;
                    let stream_work =
                        ((probe_mode || draft.stream_mode) && chat_supported).then(|| {
                            StreamControl {
                                cancel: Arc::new(AtomicBool::new(false)),
                                partial: Arc::new(Mutex::new(String::new())),
                            }
                        });
                    let request_cancel = (stream_work.is_none()
                        && matches!(&tool.adapter, Adapter::Http { .. }))
                    .then(|| Arc::new(AtomicBool::new(false)));
                    self.stream = stream_work.clone();
                    self.request_cancel = request_cancel.clone();
                    draft.output_completed = false;
                    draft.output_handoffable = false;
                    if stream_work.is_some() {
                        draft.output.clear();
                    }
                    draft.pending_chat =
                        (draft.chat_mode && !probe_mode).then(|| (binding, input.clone()));
                    let credential_id = id.clone();
                    self.token.clear();
                    let (tx, rx) = mpsc::channel();
                    self.execution_job.begin();
                    self.execution_instance = Some(id.clone());
                    self.running = Some((id, rx));
                    std::thread::spawn(move || {
                        let result = if let Some(stream) = stream_work {
                            load_secret(&tool, &credential_id, token, use_saved).and_then(
                                |secret| {
                                    crate::chat_stream::run(
                                        &tool,
                                        &input,
                                        &model,
                                        secret,
                                        messages.as_deref(),
                                        &stream.cancel,
                                        |value| {
                                            if let Ok(mut partial) = stream.partial.lock() {
                                                *partial = value;
                                            }
                                        },
                                    )
                                },
                            )
                        } else {
                            execute_with_credentials_cancellable(
                                &tool,
                                &credential_id,
                                &input,
                                &model,
                                token,
                                use_saved,
                                RequestExecution {
                                    messages: messages.as_deref(),
                                    cancelled: request_cancel.as_deref(),
                                },
                            )
                        }
                        .map_err(|e| e.to_string());
                        let _ = tx.send(result);
                    });
                }
                if self.running.is_some() {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        if let Some(stream) = &self.stream {
                            let stopping = stream.cancel.load(Ordering::Relaxed);
                            ui.label(if stopping {
                                "正在停止客户端请求…"
                            } else {
                                "流式生成中 · 最长 120 秒 · 可切换页面"
                            });
                            if ui
                                .add_enabled(!stopping, egui::Button::new("停止生成"))
                                .clicked()
                            {
                                stream.cancel.store(true, Ordering::Relaxed);
                                self.execution_job.cancelling();
                            }
                        } else if let Some(cancel) = &self.request_cancel {
                            let stopping = cancel.load(Ordering::Relaxed);
                            ui.label(if stopping {
                                "正在取消客户端等待；服务端可能仍在处理…"
                            } else {
                                "请求最多等待 30 秒；可切换页面或在任务中心取消"
                            });
                            if ui
                                .add_enabled(!stopping, egui::Button::new("取消请求"))
                                .clicked()
                            {
                                cancel.store(true, Ordering::Relaxed);
                                self.execution_job.cancelling();
                            }
                        } else {
                            ui.label("执行中，请求最多等待 30 秒；可切换页面");
                        }
                    });
                }
                ui.horizontal(|ui| {
                    ui.strong(if self.running.is_some() && self.stream.is_some() {
                        "正在生成的内容"
                    } else {
                        "执行结果"
                    });
                    if ui
                        .add_enabled(!draft.output.is_empty(), egui::Button::new("复制结果"))
                        .clicked()
                    {
                        ui.ctx().copy_text(draft.output.clone());
                        self.copied_at = Some(std::time::Instant::now());
                    }
                });
                if self
                    .copied_at
                    .is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(2))
                {
                    ui.label("已复制结果");
                }
                let mut output = draft.output.as_str();
                ui.add_sized(
                    [ui.available_width(), 260.0],
                    egui::TextEdit::multiline(&mut output).font(egui::TextStyle::Monospace),
                );
            } else {
                ui.label("工具已停用或卸载，请返回管理页。");
            }
            return;
        }
        egui::CollapsingHeader::new("安装或升级本地插件清单（JSON）")
            .default_open(self.preview.is_some() || self.catalog.is_some())
            .show(ui, |ui| {
            ui.label(
                "先预览名称、工具和请求地址，再安装或升级。安装后默认停用；升级保留上一版并要求重新授权。不会执行安装脚本或加载 DLL。",
            );
            egui::CollapsingHeader::new("插件发布者信任")
                .default_open(true)
                .show(ui, |ui| {
                    ui.label("远程更新只接受已固定 Ed25519 公钥签名的清单。请从发布者官网、代码仓库等独立渠道取得公钥，并在固定前核对下方 SHA-256 指纹；不要从待检查的同一地址取得并直接信任公钥。");
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.publisher_name_input)
                                .hint_text("发布者名称")
                                .desired_width(180.0),
                        );
                        ui.add(
                            egui::TextEdit::singleline(&mut self.publisher_key_input)
                                .hint_text("Ed25519 公钥 Base64")
                                .char_limit(128)
                                .desired_width(360.0),
                        );
                        if ui.button("固定公钥").clicked() {
                            match self.store.pin_publisher(
                                &self.publisher_name_input,
                                &self.publisher_key_input,
                            ) {
                                Ok(id) => {
                                    self.publisher_key_input.clear();
                                    self.message = format!(
                                        "已固定发布者公钥；SHA-256 指纹 {id}"
                                    );
                                }
                                Err(error) => self.message = error.to_string(),
                            }
                        }
                    });
                    if !self.publisher_key_input.trim().is_empty() {
                        match plugins::public_key_fingerprint(&self.publisher_key_input) {
                            Ok(id) => {
                                ui.label(format!(
                                    "待核对 SHA-256 指纹：{id}。请与独立可信渠道公布的完整指纹逐字比较。"
                                ));
                            }
                            Err(error) => {
                                ui.small(error.to_string());
                            }
                        }
                    }
                    let publishers = self.store.trusted_publishers.clone();
                    for publisher in publishers {
                        ui.horizontal(|ui| {
                            ui.label(format!(
                                "{} · SHA-256 {}",
                                publisher.name, publisher.id
                            ));
                            if ui.button("移除此信任").clicked() {
                                match self.store.remove_publisher(&publisher.id) {
                                    Ok(()) => {
                                        self.message = format!(
                                            "已移除发布者信任：{}；不会删除已安装插件",
                                            publisher.name
                                        );
                                    }
                                    Err(error) => self.message = error.to_string(),
                                }
                            }
                        });
                    }
                });
            egui::CollapsingHeader::new("签名插件目录")
                .default_open(self.catalog.is_some())
                .show(ui, |ui| {
                    ui.label("手动检查一个已签名的 HTTPS 插件目录。目录签名者必须已固定；目录中的每个插件还需独立固定其作者公钥，并验证插件清单的签名、ID、版本和作者指纹。此流程只提供候选预览，不会自动安装或启用。");
                    let saved_sources = self.store.catalog_sources.clone();
                    if !saved_sources.is_empty() {
                        ui.weak("已保存的目录来源只记录本机地址和签名者指纹；不会在启动时或后台自动访问。");
                        let mut selected_url = None;
                        let mut removed_source = None;
                        egui::ScrollArea::vertical()
                            .id_salt("saved-plugin-catalog-sources")
                            .max_height(120.0)
                            .show(ui, |ui| {
                                for source in &saved_sources {
                                    ui.horizontal(|ui| {
                                        if ui.button("使用").clicked() {
                                            selected_url = Some(source.url.clone());
                                        }
                                        ui.vertical(|ui| {
                                            ui.strong(&source.name);
                                            ui.small(&source.url);
                                            let trust = source
                                                .publisher_id
                                                .as_deref()
                                                .map(|id| format!("目录签名者已绑定 · {}", &id[..12]))
                                                .unwrap_or_else(|| "首次验证时绑定目录签名者".into());
                                            ui.weak(trust);
                                            let history = match source.last_verified_at.as_deref() {
                                                Some(timestamp) => {
                                                    let local = chrono::DateTime::parse_from_rfc3339(timestamp)
                                                        .map(|time| time.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string())
                                                        .unwrap_or_else(|_| "时间无效".into());
                                                    format!("上次验签：{local} · {} 项", source.last_snapshot.len())
                                                }
                                                None => "尚无验签历史".into(),
                                            };
                                            ui.small(history);
                                        });
                                        if ui.button("移除").clicked() {
                                            removed_source = Some(source.id.clone());
                                        }
                                    });
                                }
                            });
                        if let Some(url) = selected_url {
                            self.catalog_url = url;
                            self.catalog = None;
                            self.catalog_diff = None;
                        }
                        if let Some(id) = removed_source {
                            match self.store.remove_catalog_source(&id) {
                                Ok(()) => {
                                    self.message = "已移除本机保存的插件目录来源；不影响已安装插件或发布者信任".into();
                                }
                                Err(error) => self.message = error.to_string(),
                            }
                        }
                    }
                    ui.horizontal(|ui| {
                        let changed = ui
                            .add(
                                egui::TextEdit::singleline(&mut self.catalog_url)
                                    .hint_text("签名插件目录 HTTPS URL（不含查询参数）")
                                    .desired_width(520.0),
                            )
                            .changed();
                        if changed {
                            self.catalog = None;
                            self.catalog_diff = None;
                        }
                        if ui
                            .add_enabled(
                                !self.is_running(),
                                egui::Button::new("检查插件目录"),
                            )
                            .clicked()
                            && let Err(error) = self.check_catalog_source()
                        {
                            self.message = error.to_string();
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.catalog_source_name)
                                .hint_text("来源名称，例如：团队插件目录")
                                .char_limit(80)
                                .desired_width(280.0),
                        );
                        if ui
                            .add_enabled(
                                !self.is_running(),
                                egui::Button::new("保存为本机来源"),
                            )
                            .clicked()
                        {
                            match self
                                .store
                                .add_catalog_source(&self.catalog_source_name, &self.catalog_url)
                            {
                                Ok(_) => {
                                    self.catalog_source_name.clear();
                                    self.message = "已保存目录来源；只有手动点击检查时才会访问".into();
                                }
                                Err(error) => self.message = error.to_string(),
                            }
                        }
                    });
                    if self.catalog_running.is_some() {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("正在验证目录签名；可在后台任务中心取消。");
                        });
                    }
                    let mut fetch_entry = None;
                    if let Some(catalog) = self.catalog.as_ref() {
                        ui.weak(format!(
                            "目录签名者：{} · SHA-256 {} · {} 项",
                            catalog.publisher_name,
                            catalog.publisher_id,
                            catalog.catalog.entries.len()
                        ));
                        if let Some(diff) = self.catalog_diff.as_ref() {
                            egui::CollapsingHeader::new("与上次验签目录的变化")
                                .default_open(diff.had_previous_check)
                                .show(ui, |ui| {
                                    if !diff.had_previous_check {
                                        ui.weak("这是此来源首次成功验签，已建立本机比较基线。");
                                    } else {
                                        ui.label(format!(
                                            "新增 {} · 更新 {} · 移除 {}{}",
                                            diff.added.len(),
                                            diff.updated.len(),
                                            diff.removed.len(),
                                            diff.schema_version_change
                                                .map(|(old, new)| format!(" · 目录 schema {old} → {new}"))
                                                .unwrap_or_default()
                                        ));
                                        if diff.added.is_empty()
                                            && diff.updated.is_empty()
                                            && diff.removed.is_empty()
                                            && diff.schema_version_change.is_none()
                                        {
                                            ui.weak("没有发现目录条目变化。");
                                        }
                                        egui::ScrollArea::vertical()
                                            .id_salt("plugin-catalog-change-details")
                                            .max_height(140.0)
                                            .show(ui, |ui| {
                                                for entry in &diff.added {
                                                    ui.label(format!(
                                                        "+ {} · {} v{}",
                                                        entry.name, entry.plugin_id, entry.version
                                                    ));
                                                }
                                                for change in &diff.updated {
                                                    ui.label(format!(
                                                        "~ {} · {} · v{} → v{}",
                                                        change.current.name,
                                                        change.current.plugin_id,
                                                        change.previous.version,
                                                        change.current.version
                                                    ));
                                                    ui.small(format!(
                                                        "变化字段：{}",
                                                        change
                                                            .changed_fields
                                                            .iter()
                                                            .map(|field| field.label())
                                                            .collect::<Vec<_>>()
                                                            .join("、")
                                                    ));
                                                }
                                                for entry in &diff.removed {
                                                    ui.label(format!(
                                                        "− {} · {} · 已从目录移除（不会卸载本机插件）",
                                                        entry.name, entry.plugin_id
                                                    ));
                                                }
                                            });
                                        ui.small("本机变更记录可由用户清除或修改，仅用于对照；当前目录仍须独立验签。");
                                    }
                                });
                        }
                        ui.add(
                            egui::TextEdit::singleline(&mut self.catalog_search)
                                .hint_text("搜索名称、插件 ID、说明或发布者指纹")
                                .desired_width(f32::INFINITY),
                        );
                        let query = self.catalog_search.trim().to_lowercase();
                        let entries: Vec<_> = catalog.catalog.entries.iter().filter(|entry| {
                                query.is_empty()
                                    || entry.name.to_lowercase().contains(&query)
                                    || entry.plugin_id.to_lowercase().contains(&query)
                                    || entry.description.to_lowercase().contains(&query)
                                    || entry.publisher_id.to_lowercase().contains(&query)
                            })
                            .collect();
                        if entries.is_empty() {
                            ui.weak("没有匹配的插件条目。");
                        } else {
                            egui::ScrollArea::vertical()
                                .id_salt("signed-plugin-catalog-entries")
                                .max_height(260.0)
                                .show(ui, |ui| {
                                    for entry in entries {
                                        let pinned = self
                                            .store
                                            .trusted_publishers
                                            .iter()
                                            .any(|publisher| publisher.id == entry.publisher_id);
                                        let compatibility = plugins::catalog_compatibility(
                                            entry,
                                            &plugins::current_host_version(),
                                            plugins::PLUGIN_MANIFEST_SCHEMA_VERSION,
                                        );
                                        let compatible = !matches!(
                                            &compatibility,
                                            plugins::CatalogCompatibility::Incompatible(_)
                                        );
                                        ui.group(|ui| {
                                            ui.horizontal(|ui| {
                                                ui.strong(&entry.name);
                                                ui.label(format!("v{} · {}", entry.version, entry.plugin_id));
                                            });
                                            ui.label(&entry.description);
                                            ui.small(format!("作者 SHA-256：{}", entry.publisher_id));
                                            match &compatibility {
                                                plugins::CatalogCompatibility::Compatible => {
                                                    ui.small(format!(
                                                        "兼容：ZiDevTools {}（当前 {}）· 插件清单 schema {}",
                                                        entry.host_app_version_req.as_deref().unwrap_or("*"),
                                                        plugins::current_host_version(),
                                                        entry.plugin_manifest_schema_version.unwrap_or(plugins::PLUGIN_MANIFEST_SCHEMA_VERSION)
                                                    ));
                                                }
                                                plugins::CatalogCompatibility::LegacyUnspecified => {
                                                    ui.weak("旧版目录未声明宿主兼容范围；取得清单后仍需逐项人工复核。");
                                                }
                                                plugins::CatalogCompatibility::Incompatible(reason) => {
                                                    ui.weak(format!("当前应用不能获取：{reason}"));
                                                }
                                            }
                                            if pinned {
                                                ui.horizontal(|ui| {
                                                    ui.label("✓ 作者公钥已固定");
                                                    if ui
                                                        .add_enabled(
                                                            !self.is_running() && compatible,
                                                            egui::Button::new("获取并验证清单"),
                                                        )
                                                        .clicked()
                                                    {
                                                        fetch_entry = Some(entry.clone());
                                                    }
                                                });
                                            } else {
                                                ui.weak("作者公钥尚未固定；请通过独立渠道核对上方完整指纹，再到“插件发布者信任”中固定。");
                                            }
                                        });
                                    }
                                });
                        }
                    }
                    if let Some(entry) = fetch_entry
                        && let Err(error) = self.check_catalog_entry(entry)
                    {
                        self.message = error.to_string();
                    }
                });
            ui.label("更新检查要求签名匹配本机已固定的发布者公钥；拒绝重定向、凭据、查询参数和超过 256 KiB 的响应。验证通过后清单仍先进入差异预览，不会自动安装或启用。HTTP 仅允许本机回环地址。");
            ui.horizontal(|ui| {
                let changed = ui.add(
                    egui::TextEdit::singleline(&mut self.update_url)
                        .hint_text("插件更新清单 HTTPS URL（不含查询参数）")
                        .desired_width(520.0),
                ).changed();
                if changed
                    && self
                        .preview_origin
                        .as_deref()
                        .is_some_and(|origin| origin.starts_with("用户指定的 HTTPS 更新源"))
                {
                    self.preview = None;
                    self.preview_origin = None;
                }
                if ui.add_enabled(
                    !self.is_running(),
                    egui::Button::new("检查更新清单"),
                ).clicked() {
                    if let Err(error) = self.check_update_source() {
                        self.message = error.to_string();
                    }
                }
            });
            if self.update_running.is_some() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("正在取得清单；可在后台任务中心取消。远端清单仍须审核。" );
                });
            }
            ui.horizontal(|ui| {
                let changed = ui.add(
                    egui::TextEdit::singleline(&mut self.path)
                        .hint_text("插件 JSON 文件的完整路径")
                        .desired_width(420.0),
                ).changed();
                if changed && self.preview_origin.as_deref() == Some("本机选择的 JSON 清单") {
                    self.preview = None;
                    self.preview_origin = None;
                }
                if ui.add_enabled(!self.is_running(), egui::Button::new("读取预览")).clicked() {
                    match plugins::read_manifest(std::path::Path::new(
                        self.path.trim().trim_matches('"'),
                    ))
                    .and_then(|b| {
                        plugins::parse(&b)?;
                        Ok(b)
                    }) {
                        Ok(bytes) => {
                            self.preview = Some(bytes);
                            self.preview_origin = Some("本机选择的 JSON 清单".into());
                            self.message.clear();
                        }
                        Err(e) => {
                            self.preview = None;
                            self.preview_origin = None;
                            self.message = e.to_string();
                        }
                    }
                }
            });
            if let Some(bytes) = self.preview.clone()
                && let Ok(m) = plugins::parse(&bytes)
            {
                if let Some(origin) = &self.preview_origin {
                    ui.small(format!("清单来源：{origin}"));
                }
                ui.strong(format!("{} · v{} · {}", m.name, m.version, m.id));
                ui.label(m.description);
                for tool in &m.tools {
                    ui.label(format!("{} · {}", tool.name, tool.category));
                    show_tool_aliases(ui, &m.id, tool, "manifest-preview");
                    if let Adapter::Http { url, method, .. } = &tool.adapter {
                        ui.label(format!("网络权限：{method} {url}"));
                    }
                }
                let existing = self.store.packages.iter().find(|p| p.manifest.id == m.id);
                if let Some(current) = existing {
                    match self.store.update_plan(&bytes) {
                        Ok(plan) => {
                            ui.separator();
                            ui.label(format!(
                                "升级预览：v{} → v{}",
                                plan.current_version, plan.candidate_version
                            ));
                            ui.label(format!(
                                "新增工具：{}；移除工具：{}；定义变化：{}",
                                if plan.added_tools.is_empty() { "无".to_owned() } else { plan.added_tools.join("、") },
                                if plan.removed_tools.is_empty() { "无".to_owned() } else { plan.removed_tools.join("、") },
                                if plan.changed_tools.is_empty() { "无".to_owned() } else { plan.changed_tools.join("、") }
                            ));
                            if !plan.tool_version_changes.is_empty() {
                                ui.strong("独立工具版本变化");
                                for change in &plan.tool_version_changes {
                                    let previous = change
                                        .previous_version
                                        .as_deref()
                                        .map_or_else(|| "未声明".to_owned(), |v| format!("v{v}"));
                                    let candidate = change
                                        .candidate_version
                                        .as_deref()
                                        .map_or_else(|| "未声明".to_owned(), |v| format!("v{v}"));
                                    ui.small(format!(
                                        "{} · {}：{previous} → {candidate}",
                                        change.name, change.tool_id
                                    ));
                                    if change.history_changed && change.added_history.is_empty() {
                                        ui.weak("版本记录内容有变化，但没有新增历史版本项。");
                                    }
                                    for record in &change.added_history {
                                        ui.small(format!(
                                            "新增记录：v{} · {} · {}",
                                            record.version, record.date, record.changes
                                        ));
                                    }
                                }
                            }
                            if !plan.permission_changes.is_empty() {
                                ui.colored_label(
                                    ui.visuals().warn_fg_color,
                                    format!("网络/操作声明变化：{}", plan.permission_changes.join("、")),
                                );
                            } else {
                                ui.label("网络/操作声明无变化");
                            }
                            ui.label("升级会保存当前版本以供回滚，并停用新版本；核对新声明后需重新启用。此处只导入本机清单，不会自动联网查找更新。");
                            if ui.add_enabled(
                                !self.is_running(),
                                egui::Button::new(format!("确认升级到 v{}（升级后停用）", plan.candidate_version)),
                            ).clicked() {
                                self.message = match self.store.update(&bytes) {
                                    Ok(()) => {
                                        self.preview = None;
                                        "插件已升级并停用；核对清单后可重新启用，也可回滚上一版本".into()
                                    }
                                    Err(error) => error.to_string(),
                                };
                            }
                        }
                        Err(error) => {
                            ui.colored_label(
                                egui::Color32::LIGHT_RED,
                                format!("无法升级已安装的 {} v{}：{error}", current.manifest.name, current.manifest.version),
                            );
                        }
                    }
                } else if ui.button("安装此清单（默认停用）").clicked() {
                    self.message = match self.store.install(&bytes) {
                        Ok(()) => {
                            self.preview = None;
                            "安装完成，请核对地址后启用".into()
                        }
                        Err(e) => e.to_string(),
                    };
                }
            }
        });
        egui::CollapsingHeader::new("官方示例 · 无需下载即可安装")
            .default_open(self.catalog.is_none())
            .show(ui, |ui| {
                for (name, bytes) in [
                    (
                        "本地文本配方",
                        include_bytes!("../plugins-examples/local-text.json").as_slice(),
                    ),
                    (
                        "Ollama 连接器",
                        include_bytes!("../plugins-examples/ollama.json").as_slice(),
                    ),
                    (
                        "OpenAI 兼容连接器",
                        include_bytes!("../plugins-examples/openai-compatible.json").as_slice(),
                    ),
                ] {
                    if ui.button(format!("安装 {name}")).clicked() {
                        self.message = match self.store.install(bytes) {
                            Ok(()) => "已安装，默认停用；请核对声明后启用".into(),
                            Err(e) => e.to_string(),
                        };
                    }
                }
            });
        if !self.message.is_empty() {
            ui.label(&self.message);
        }
        for error in &self.store.errors {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        ui.add_space(14.0);
        if ui
            .add_enabled(
                self.running.is_none() && self.discovery_running.is_none(),
                egui::Button::new("刷新已安装清单"),
            )
            .clicked()
        {
            self.store = Store::load(self.store.root.clone());
            self.clear_token();
        }
        ui.strong(format!("已安装 {} 个插件", self.store.packages.len()));
        enum Action {
            Enable(String, bool),
            Remove(String),
            ConfirmRollback(String),
            Open(String),
        }
        let mut action = None;
        for package in &self.store.packages {
            let m = &package.manifest;
            egui::Frame::group(ui.style())
                .fill(ui.visuals().faint_bg_color)
                .corner_radius(12)
                .inner_margin(14.0)
                .show(ui, |ui| {
                    ui.set_min_width((ui.available_width() - 4.0).max(0.0));
                    ui.horizontal_wrapped(|ui| {
                        ui.strong(&m.name);
                        ui.label(format!("v{} · {} 个工具", m.version, m.tools.len()));
                        ui.label(if package.enabled {
                            "已启用"
                        } else {
                            "已停用 / 内容变更后需重新启用"
                        });
                    });
                    ui.label(&m.description);
                    for tool in &m.tools {
                        ui.small(format!(
                            "工具版本：{} · {}",
                            tool.name,
                            tool.version.as_deref().map_or_else(
                                || "未声明".to_owned(),
                                |version| format!("v{version}")
                            )
                        ));
                        show_tool_aliases(ui, &m.id, tool, "installed-plugin");
                        if !tool.version_history.is_empty() {
                            ui.collapsing(format!("{} · 版本记录", tool.name), |ui| {
                                for record in &tool.version_history {
                                    ui.small(format!(
                                        "v{} · {} · {}",
                                        record.version, record.date, record.changes
                                    ));
                                }
                            });
                        }
                        if let Adapter::Http { url, method, .. } = &tool.adapter {
                            ui.label(format!("网络声明：{method} {url}"));
                        }
                    }
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .add_enabled(
                                self.running.is_none() && self.discovery_running.is_none(),
                                egui::Button::new(if package.enabled {
                                    "停用"
                                } else if m
                                    .tools
                                    .iter()
                                    .any(|t| matches!(t.adapter, Adapter::Http { .. }))
                                {
                                    "启用并允许上述网络请求"
                                } else {
                                    "启用本地处理插件"
                                }),
                            )
                            .clicked()
                        {
                            action = Some(Action::Enable(m.id.clone(), !package.enabled));
                        }
                        if ui
                            .add_enabled(
                                self.running.is_none() && self.discovery_running.is_none(),
                                egui::Button::new("卸载"),
                            )
                            .clicked()
                        {
                            self.pending_remove = Some(m.id.clone());
                        }
                        if let Some(version) = self.store.available_rollback_version(&m.id)
                            && ui
                                .add_enabled(
                                    self.running.is_none() && self.discovery_running.is_none(),
                                    egui::Button::new(format!("回滚到 v{version}")),
                                )
                                .clicked()
                        {
                            self.pending_rollback = Some(m.id.clone());
                        }
                    });
                    if self.pending_remove.as_deref() == Some(&m.id) {
                        ui.horizontal(|ui| {
                            ui.label("移除清单与启用状态？");
                            if ui.button("确认卸载").clicked() {
                                action = Some(Action::Remove(m.id.clone()));
                            }
                            if ui.button("取消").clicked() {
                                self.pending_remove = None;
                            }
                        });
                    }
                    if self.pending_rollback.as_deref() == Some(&m.id) {
                        ui.horizontal_wrapped(|ui| {
                            let version = self
                                .store
                                .available_rollback_version(&m.id)
                                .unwrap_or_else(|| "未知".into());
                            ui.label(format!(
                                "恢复 v{version} 并停用插件？重新启用前需再次核对其声明。"
                            ));
                            if ui.button("确认回滚").clicked() {
                                action = Some(Action::ConfirmRollback(m.id.clone()));
                            }
                            if ui.button("取消").clicked() {
                                self.pending_rollback = None;
                            }
                        });
                    }
                    if package.enabled {
                        for t in &m.tools {
                            if ui.button(format!("打开 {} →", t.name)).clicked() {
                                action = Some(Action::Open(format!("plugin:{}/{}", m.id, t.id)));
                            }
                        }
                    }
                });
            ui.add_space(8.0);
        }
        match action {
            Some(Action::Enable(id, enabled)) => {
                self.message = match self.store.set_enabled(&id, enabled) {
                    Ok(()) => if enabled {
                        "插件已启用"
                    } else {
                        "插件已停用"
                    }
                    .into(),
                    Err(e) => e.to_string(),
                };
                self.clear_token();
            }
            Some(Action::Remove(id)) => {
                match self.store.uninstall(&id) {
                    Ok(()) => {
                        self.drafts
                            .retain(|key, _| !key.starts_with(&format!("plugin:{id}/")));
                        self.message = "插件已卸载".into();
                    }
                    Err(e) => self.message = e.to_string(),
                }
                self.clear_token();
                self.pending_remove = None;
            }
            Some(Action::ConfirmRollback(id)) => {
                self.message = match self.store.rollback_version(&id) {
                    Ok(()) => "已回滚上一版本并停用；重新核对声明后再启用".into(),
                    Err(error) => error.to_string(),
                };
                self.pending_rollback = None;
                self.clear_token();
            }
            Some(Action::Open(id)) => self.select(&id),
            None => {}
        }
        if self.running.is_some() {
            ui.label("有工具正在运行，完成后可停用或卸载。");
        }
    }
}

fn profiles_ui(
    ui: &mut egui::Ui,
    settings: &mut crate::plugin_settings::Settings,
    draft: &mut Draft,
    tool: &plugins::PluginTool,
) -> bool {
    use crate::plugin_settings::{Connection, Profile, request_shape};
    let Some(shape) = request_shape(tool) else {
        return false;
    };
    let mut applied = false;
    let selected_name = draft
        .profile_id
        .as_deref()
        .and_then(|id| settings.profile(id))
        .map_or("选择兼容档案", |p| p.name.as_str())
        .to_owned();
    ui.horizontal_wrapped(|ui| {
        ui.label("连接档案");
        egui::ComboBox::from_id_salt("connection-profile")
            .selected_text(selected_name)
            .width(200.0)
            .show_ui(ui, |ui| {
                let mut profiles: Vec<_> = settings
                    .profiles()
                    .filter(|(_, p)| p.shape == shape)
                    .collect();
                profiles.sort_by(|a, b| a.1.name.cmp(&b.1.name));
                if profiles.is_empty() {
                    ui.weak("尚无兼容档案，可保存当前连接");
                }
                for (id, profile) in profiles {
                    if ui
                        .selectable_label(draft.profile_id.as_ref() == Some(id), &profile.name)
                        .on_hover_text(&profile.connection.endpoint)
                        .clicked()
                    {
                        draft.profile_id = Some(id.clone());
                        draft.profile_name = profile.name.clone();
                        draft.profile_delete = false;
                    }
                }
            });
        let selected = draft
            .profile_id
            .as_deref()
            .and_then(|id| settings.profile(id));
        if ui
            .add_enabled(
                selected.is_some_and(|p| p.shape == shape),
                egui::Button::new("应用到当前工具"),
            )
            .clicked()
        {
            if let Some(value) = selected.and_then(|p| p.for_tool(tool).ok()) {
                draft.endpoint = value.endpoint;
                draft.model = value.model;
                draft.settings_message =
                    "已应用档案；请核对目标并重新选择认证，保存连接设置后可在重启时复用".into();
                applied = true;
            }
        }
        ui.toggle_value(&mut draft.profile_editor, "管理档案");
    });
    if draft.profile_editor {
        ui.group(|ui| {
            ui.label("档案只包含接口与模型；应用时复制配置，不自动发送请求。仅显示请求和响应格式兼容的档案。");
            ui.weak("可在下方运行区执行“测试模型生成”：只向当前地址发送固定提示“只回复 OK”，可能产生服务费用；测试结果不会加入会话或工具接力。");
            ui.horizontal_wrapped(|ui| {
                ui.label("档案名称");
                ui.add(egui::TextEdit::singleline(&mut draft.profile_name).hint_text("例如：本机推理 / 开发环境"));
                let profile = Profile { name: draft.profile_name.clone(), shape: shape.clone(), connection: Connection { endpoint: draft.endpoint.clone(), model: draft.model.clone() } };
                let valid = !profile.name.trim().is_empty() && profile.connection.validate().is_ok() && settings.error.is_none();
                if ui.add_enabled(valid, egui::Button::new("另存新档案")).clicked() {
                    match settings.save_profile(None, profile.clone()) {
                        Ok(id) => { draft.profile_id = Some(id); draft.profile_delete = false; draft.settings_message = "档案已创建；未保存认证信息".into(); }
                        Err(e) => draft.settings_message = e.to_string(),
                    }
                }
                if ui.add_enabled(valid && draft.profile_id.is_some(), egui::Button::new("更新选中档案")).clicked() {
                    draft.settings_message = match settings.save_profile(draft.profile_id.as_deref(), profile) {
                        Ok(_) => "档案已更新；已应用到其他工具的设置保持原值".into(),
                        Err(e) => e.to_string(),
                    };
                }
                if ui.add_enabled(draft.profile_id.is_some() && settings.error.is_none(), egui::Button::new("删除档案")).clicked() { draft.profile_delete = true; }
            });
            if draft.profile_delete {
                ui.horizontal_wrapped(|ui| {
                    ui.label("删除选中档案？已应用的工具设置与系统凭据会保留。");
                    if ui.button("确认删除").clicked() {
                        if let Some(id) = &draft.profile_id {
                            match settings.delete_profile(id) {
                                Ok(()) => { draft.profile_id = None; draft.profile_name.clear(); draft.settings_message = "档案已删除".into(); }
                                Err(e) => draft.settings_message = e.to_string(),
                            }
                        }
                        draft.profile_delete = false;
                    }
                    if ui.button("取消").clicked() { draft.profile_delete = false; }
                });
            }
        });
    }
    applied
}

fn load_secret(
    tool: &plugins::PluginTool,
    id: &str,
    temporary: String,
    saved: bool,
) -> anyhow::Result<Option<crate::credentials::Secret>> {
    let secret = if saved {
        let Adapter::Http { url, method, .. } = &tool.adapter else {
            anyhow::bail!("本地配方不使用网络凭据");
        };
        let target = crate::credentials::Target::plugin(id, method, url)?;
        Some(crate::credentials::read(&target)?.ok_or_else(|| {
            anyhow::anyhow!("当前工具和接口地址没有已保存凭据，请先保存或取消勾选")
        })?)
    } else if temporary.trim().is_empty() {
        None
    } else {
        Some(crate::credentials::Secret::new(temporary)?)
    };
    Ok(secret)
}

#[cfg(test)]
fn execute_with_credentials(
    tool: &plugins::PluginTool,
    id: &str,
    input: &str,
    model: &str,
    temporary: String,
    saved: bool,
    messages: Option<&[serde_json::Value]>,
) -> anyhow::Result<String> {
    execute_with_credentials_cancellable(
        tool,
        id,
        input,
        model,
        temporary,
        saved,
        RequestExecution {
            messages,
            cancelled: None,
        },
    )
}

struct RequestExecution<'a> {
    messages: Option<&'a [serde_json::Value]>,
    cancelled: Option<&'a AtomicBool>,
}

fn execute_with_credentials_cancellable(
    tool: &plugins::PluginTool,
    id: &str,
    input: &str,
    model: &str,
    temporary: String,
    saved: bool,
    request: RequestExecution<'_>,
) -> anyhow::Result<String> {
    let secret = load_secret(tool, id, temporary, saved)?;
    let token = secret.as_ref().map_or("", |s| s.expose());
    let redact = |text: String| {
        if token.is_empty() {
            text
        } else {
            let escaped = serde_json::to_string(token).unwrap_or_default();
            text.replace(token, "[REDACTED]").replace(
                escaped
                    .get(1..escaped.len().saturating_sub(1))
                    .unwrap_or(token),
                "[REDACTED]",
            )
        }
    };
    plugins::execute_with_context_cancellable(
        tool,
        input,
        model,
        token,
        request.messages,
        request.cancelled,
    )
    .map(redact)
    .map_err(|e| anyhow::anyhow!(redact(e.to_string())))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn plugin_handoff_requires_a_completed_selected_output() {
        let root = std::env::temp_dir().join(format!("zi-plugin-handoff-{}", uuid::Uuid::new_v4()));
        let mut state = PluginState::new(root.clone());
        let selected = "fixture-plugin/chat".to_owned();
        state.selected = Some(selected.clone());
        let draft = state.drafts.entry(selected.clone()).or_default();
        draft.output = "final plugin answer".into();
        draft.output_completed = true;
        draft.output_handoffable = true;

        assert_eq!(
            state.handoff_result(),
            Some(("插件最终结果".into(), "final plugin answer".into()))
        );
        state.drafts.get_mut(&selected).unwrap().output_completed = false;
        assert!(state.handoff_result().is_none());
        state.drafts.get_mut(&selected).unwrap().output_completed = true;
        state.drafts.get_mut(&selected).unwrap().output_handoffable = false;
        assert!(state.handoff_result().is_none());
        state.drafts.get_mut(&selected).unwrap().output_handoffable = true;
        let (_sender, receiver) = mpsc::channel();
        state.running = Some((selected, receiver));
        assert!(state.handoff_result().is_none());

        drop(state);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn completed_model_probe_restores_input_and_cannot_enter_chat_or_handoff() {
        let root = std::env::temp_dir().join(format!("zi-plugin-probe-{}", uuid::Uuid::new_v4()));
        let mut state = PluginState::new(root.clone());
        let selected = "fixture-plugin/chat".to_owned();
        state.selected = Some(selected.clone());
        let draft = state.drafts.entry(selected.clone()).or_default();
        draft.input = "original draft".into();
        draft.probe_previous_input = Some("original draft".into());
        draft.probe_mode = true;
        let (sender, receiver) = mpsc::channel();
        sender.send(Ok("OK".into())).unwrap();
        state.running = Some((selected.clone(), receiver));

        assert_eq!(state.poll().as_deref(), Some("插件执行完成"));
        let draft = state.drafts.get(&selected).unwrap();
        assert_eq!(draft.input, "original draft");
        assert_eq!(draft.output, "OK");
        assert!(draft.output_completed);
        assert!(!draft.output_handoffable);
        assert!(draft.conversation.turns().is_empty());
        assert!(draft.chat_message.contains("模型生成测试成功"));
        assert!(state.handoff_result().is_none());

        drop(state);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn remote_update_manifest_stops_at_candidate_preview_without_installing() {
        use std::{
            io::{Read, Write},
            net::TcpListener,
            time::{Duration, Instant},
        };
        const MANIFEST: &[u8] = include_bytes!("../plugins-examples/local-text.json");
        let (signed_envelope, publisher) = plugins::signed_test_envelope(MANIFEST, "CI fixture");
        let fingerprint = publisher.id.clone();
        let root =
            std::env::temp_dir().join(format!("zi-plugin-update-preview-{}", uuid::Uuid::new_v4()));
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let source = format!("http://{}/manifest.json", listener.local_addr().unwrap());
        let response_body = signed_envelope.clone();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = [0; 2048];
            let size = stream.read(&mut request).unwrap();
            assert!(String::from_utf8_lossy(&request[..size]).starts_with("GET /manifest.json "));
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                response_body.len()
            )
            .unwrap();
            stream.write_all(&response_body).unwrap();
        });
        let mut state = PluginState::new(root.clone());
        state
            .store
            .pin_publisher(&publisher.name, &publisher.public_key)
            .unwrap();
        state.update_url = source;
        state.check_update_source().unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while state.update_running.is_some() && Instant::now() < deadline {
            state.poll();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(state.update_running.is_none());
        assert_eq!(state.preview.as_deref(), Some(MANIFEST));
        let origin = state.preview_origin.as_deref().unwrap();
        assert!(origin.contains("签名已验证"));
        assert!(origin.contains(&fingerprint));
        assert!(state.store.packages.is_empty());
        assert!(state.message.contains("签名验证通过") && state.message.contains("再手动确认"));
        server.join().unwrap();
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn remote_update_check_is_generic_cancellable_and_discards_changed_source() {
        use std::{
            io::Read,
            net::TcpListener,
            time::{Duration, Instant},
        };
        let root =
            std::env::temp_dir().join(format!("zi-plugin-update-ui-{}", uuid::Uuid::new_v4()));
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let source = format!("http://{}/private/path", listener.local_addr().unwrap());
        let (accepted_tx, accepted_rx) = mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 512];
            loop {
                let size = stream.read(&mut buffer).unwrap();
                assert!(size > 0);
                request.extend_from_slice(&buffer[..size]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            accepted_tx.send(()).unwrap();
            let mut byte = [0; 1];
            stream.read(&mut byte).unwrap() == 0
        });
        let mut state = PluginState::new(root.clone());
        let (_, publisher) = plugins::signed_test_envelope(
            include_bytes!("../plugins-examples/local-text.json"),
            "CI fixture",
        );
        state
            .store
            .pin_publisher(&publisher.name, &publisher.public_key)
            .unwrap();
        state.update_url = source.clone();
        state.check_update_source().unwrap();
        let row = state
            .task_snapshots()
            .into_iter()
            .find(|row| row.key == "plugin-update-check")
            .unwrap();
        assert!(row.cancellable);
        assert!(!row.summary.contains("private"));
        assert!(!row.instance.as_deref().unwrap().contains("127.0.0.1"));
        accepted_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        state.update_url = "https://updates.example.invalid/changed.json".into();
        assert!(state.cancel_task(row.instance.as_deref().unwrap(), row.generation));
        let deadline = Instant::now() + Duration::from_secs(2);
        while state.update_running.is_some() && Instant::now() < deadline {
            state.poll();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(state.update_running.is_none());
        assert!(state.preview.is_none());
        assert!(!state.cancel_task(row.instance.as_deref().unwrap(), row.generation));
        assert!(server.join().unwrap());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn signed_catalog_fetch_stays_preview_only_then_verifies_selected_manifest() {
        use std::{
            io::{Read, Write},
            net::TcpListener,
            time::{Duration, Instant},
        };
        fn serve_once(body: Vec<u8>, path: &'static str) -> (String, std::thread::JoinHandle<()>) {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let source = format!("http://{}/{}", listener.local_addr().unwrap(), path);
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut request = [0; 2048];
                let size = stream.read(&mut request).unwrap();
                assert!(
                    String::from_utf8_lossy(&request[..size]).starts_with(&format!("GET /{path} "))
                );
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                stream.write_all(&body).unwrap();
            });
            (source, server)
        }
        const MANIFEST: &[u8] = include_bytes!("../plugins-examples/local-text.json");
        let (signed_manifest, author) = plugins::signed_test_envelope(MANIFEST, "Plugin author");
        let (manifest_url, manifest_server) = serve_once(signed_manifest.clone(), "manifest.json");
        let catalog_bytes = serde_json::to_vec(&plugins::PluginCatalog {
            schema_version: 2,
            entries: vec![plugins::CatalogEntry {
                plugin_id: "local-text".into(),
                name: "本地文本工具".into(),
                version: plugins::parse(MANIFEST).unwrap().version,
                description: "目录签名和逐项清单验签测试".into(),
                manifest_url,
                publisher_id: author.id.clone(),
                host_app_version_req: Some(">=0.82.0-dev.1, <0.83.0".into()),
                plugin_manifest_schema_version: Some(1),
            }],
        })
        .unwrap();
        let (signed_catalog, catalog_signer) =
            plugins::signed_test_catalog_envelope(&catalog_bytes, "Catalog signer");
        let (catalog_url, catalog_server) = serve_once(signed_catalog, "catalog.json");
        let root =
            std::env::temp_dir().join(format!("zi-plugin-signed-catalog-{}", uuid::Uuid::new_v4()));
        let mut state = PluginState::new(root.clone());
        state
            .store
            .pin_publisher(&catalog_signer.name, &catalog_signer.public_key)
            .unwrap();
        state
            .store
            .pin_publisher(&author.name, &author.public_key)
            .unwrap();
        state.catalog_url = catalog_url.clone();
        state
            .store
            .add_catalog_source("CI 插件目录", &catalog_url)
            .unwrap();
        state.check_catalog_source().unwrap();
        let task = state
            .task_snapshots()
            .into_iter()
            .find(|row| row.key == "plugin-catalog-check")
            .unwrap();
        assert!(task.cancellable);
        assert!(!task.summary.contains("catalog"));
        assert!(!task.instance.as_deref().unwrap().contains("127.0.0.1"));
        let deadline = Instant::now() + Duration::from_secs(3);
        while state.catalog_running.is_some() && Instant::now() < deadline {
            state.poll();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(state.catalog_running.is_none());
        let catalog = state.catalog.clone().unwrap();
        assert_eq!(catalog.catalog.entries.len(), 1);
        assert_eq!(catalog.publisher_id, catalog_signer.id);
        assert_eq!(
            state.store.catalog_sources[0].publisher_id.as_deref(),
            Some(catalog_signer.id.as_str())
        );
        assert_eq!(
            plugins::Store::load(root.clone()).catalog_sources[0]
                .publisher_id
                .as_deref(),
            Some(catalog_signer.id.as_str())
        );
        assert!(
            state
                .catalog_diff
                .as_ref()
                .is_some_and(|diff| !diff.had_previous_check)
        );
        assert_eq!(state.store.catalog_sources[0].last_snapshot.len(), 1);
        assert!(state.store.packages.is_empty());
        assert!(state.preview.is_none());
        catalog_server.join().unwrap();

        let selected_entry = catalog.catalog.entries.into_iter().next().unwrap();
        state.check_catalog_entry(selected_entry.clone()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while state.update_running.is_some() && Instant::now() < deadline {
            state.poll();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(state.update_running.is_none());
        assert_eq!(state.preview.as_deref(), Some(MANIFEST));
        assert!(state.message.contains("再手动确认"));
        assert!(state.store.packages.is_empty());
        manifest_server.join().unwrap();

        let (mismatch_url, mismatch_server) = serve_once(signed_manifest, "mismatch.json");
        let mut mismatched_entry = selected_entry;
        mismatched_entry.version = "9.9.9".into();
        mismatched_entry.manifest_url = mismatch_url;
        state.check_catalog_entry(mismatched_entry).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while state.update_running.is_some() && Instant::now() < deadline {
            state.poll();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(state.update_running.is_none());
        assert!(state.preview.is_none());
        assert!(
            state
                .message
                .contains("与已签名目录中的 ID、版本或发布者不一致")
        );
        assert!(state.store.packages.is_empty());
        mismatch_server.join().unwrap();
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn saved_catalog_source_rejects_a_changed_directory_signer() {
        use std::{
            io::{Read, Write},
            net::TcpListener,
            time::{Duration, Instant},
        };
        let payload = br#"{"schema_version":1,"entries":[]}"#;
        let (first_envelope, first_signer) =
            plugins::signed_test_catalog_envelope(payload, "First catalog signer");
        let (replacement_envelope, replacement_signer) =
            plugins::signed_test_catalog_envelope(payload, "Replacement catalog signer");
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let source = format!("http://{}/catalog.json", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            for body in [first_envelope, replacement_envelope] {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0; 512];
                loop {
                    let size = stream.read(&mut buffer).unwrap();
                    assert!(size > 0);
                    request.extend_from_slice(&buffer[..size]);
                    if request.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                assert!(String::from_utf8_lossy(&request).starts_with("GET /catalog.json "));
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                stream.write_all(&body).unwrap();
            }
        });
        let root = std::env::temp_dir().join(format!(
            "zi-plugin-catalog-source-pin-{}",
            uuid::Uuid::new_v4()
        ));
        let mut state = PluginState::new(root.clone());
        for signer in [&first_signer, &replacement_signer] {
            state
                .store
                .pin_publisher(&signer.name, &signer.public_key)
                .unwrap();
        }
        state
            .store
            .add_catalog_source("Pinned source", &source)
            .unwrap();
        state.catalog_url = source;

        for attempt in 0..2 {
            state.check_catalog_source().unwrap();
            let deadline = Instant::now() + Duration::from_secs(3);
            while state.catalog_running.is_some() && Instant::now() < deadline {
                state.poll();
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(state.catalog_running.is_none());
            if attempt == 0 {
                assert_eq!(
                    state.store.catalog_sources[0].publisher_id.as_deref(),
                    Some(first_signer.id.as_str())
                );
            } else {
                assert!(state.catalog.is_none());
                assert!(
                    state
                        .message
                        .contains("签名者与首次验证时已固定的签名者不一致")
                );
            }
        }
        server.join().unwrap();
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn signed_catalog_check_can_be_cancelled_from_its_generic_task_row() {
        use std::{
            io::Read,
            net::TcpListener,
            time::{Duration, Instant},
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/catalog.json", listener.local_addr().unwrap());
        let (accepted_tx, accepted_rx) = mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 512];
            loop {
                let size = stream.read(&mut buffer).unwrap();
                assert!(size > 0);
                request.extend_from_slice(&buffer[..size]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            accepted_tx.send(()).unwrap();
            let mut byte = [0; 1];
            stream.read(&mut byte).unwrap() == 0
        });
        let root =
            std::env::temp_dir().join(format!("zi-plugin-catalog-cancel-{}", uuid::Uuid::new_v4()));
        let mut state = PluginState::new(root.clone());
        let (_, signer) = plugins::signed_test_catalog_envelope(
            br#"{"schema_version":1,"entries":[]}"#,
            "Catalog signer",
        );
        state
            .store
            .pin_publisher(&signer.name, &signer.public_key)
            .unwrap();
        state.catalog_url = url.clone();
        state
            .store
            .add_catalog_source("Cancelled source", &url)
            .unwrap();
        state.check_catalog_source().unwrap();
        let task = state
            .task_snapshots()
            .into_iter()
            .find(|row| row.key == "plugin-catalog-check")
            .unwrap();
        accepted_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(state.cancel_task(task.instance.as_deref().unwrap(), task.generation));
        let deadline = Instant::now() + Duration::from_secs(2);
        while state.catalog_running.is_some() && Instant::now() < deadline {
            state.poll();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(state.catalog_running.is_none());
        assert!(state.catalog.is_none());
        assert!(state.store.catalog_sources[0].publisher_id.is_none());
        assert!(state.store.catalog_sources[0].last_verified_at.is_none());
        let terminal = state
            .task_snapshots()
            .into_iter()
            .find(|row| row.key == "plugin-catalog-check")
            .unwrap();
        assert_eq!(terminal.phase, Phase::Cancelled);
        assert!(!state.cancel_task(task.instance.as_deref().unwrap(), task.generation));
        assert!(server.join().unwrap());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn applying_prompt_never_runs_and_only_matching_tool_can_restore_parameters() {
        let mut draft = Draft {
            model: "old-model".into(),
            chat_mode: true,
            ..Default::default()
        };
        draft.conversation.bind("shape:old-model".into());
        draft
            .conversation
            .complete("old".into(), "answer".into())
            .unwrap();
        let options = crate::prompt_template::RunOptions {
            tool_id: "plugin:other/chat".into(),
            model: "new-model".into(),
            stream: true,
            multi_turn: false,
        };
        draft.apply_prompt(
            prompts::Apply {
                text: "rendered".into(),
                options: Some(options.clone()),
            },
            "plugin:here/chat",
            "shape",
        );
        assert_eq!(draft.input, "rendered");
        assert_eq!(draft.model, "old-model");
        assert_eq!(draft.conversation.turns().len(), 1);
        draft.apply_prompt(
            prompts::Apply {
                text: "rendered again".into(),
                options: None,
            },
            "plugin:here/chat",
            "shape",
        );
        assert_eq!(draft.conversation.turns().len(), 1);
        draft.apply_prompt(
            prompts::Apply {
                text: "new input".into(),
                options: Some(crate::prompt_template::RunOptions {
                    tool_id: "plugin:here/chat".into(),
                    ..options
                }),
            },
            "plugin:here/chat",
            "shape",
        );
        assert_eq!(draft.model, "new-model");
        assert!(draft.stream_mode);
        assert!(!draft.chat_mode);
        assert!(draft.conversation.turns().is_empty());
        assert_eq!(draft.conversation.binding(), "shape:new-model");
    }
    #[test]
    fn changing_binding_discards_preview_even_when_current_history_is_empty() {
        let mut draft = Draft::default();
        draft.bind_conversation("old".into());
        let mut candidate = crate::conversation::Conversation::default();
        candidate.bind("old".into());
        candidate.complete("q".into(), "a".into()).unwrap();
        draft.library_preview = Some(candidate);
        assert!(draft.conversation.turns().is_empty());
        draft.bind_conversation("new".into());
        assert!(draft.library_preview.is_none());
        assert!(draft.conversation.turns().is_empty());
        assert_eq!(draft.conversation.binding(), "new");
    }
    #[test]
    fn stopped_stream_retains_partial_and_never_commits_late_success() {
        let mut state = PluginState::new(
            std::env::temp_dir().join(format!("zi-stream-ui-{}", uuid::Uuid::new_v4())),
        );
        let draft = state.drafts.entry("chat".into()).or_default();
        draft.conversation.bind("binding".into());
        draft.input = "question".into();
        draft.pending_chat = Some(("binding".into(), "question".into()));
        let (sender, receiver) = mpsc::channel();
        state.running = Some(("chat".into(), receiver));
        state.stream = Some(StreamControl {
            cancel: Arc::new(AtomicBool::new(false)),
            partial: Arc::new(Mutex::new("partial text".into())),
        });
        assert!(state.poll().is_none());
        assert_eq!(state.drafts["chat"].output, "partial text");
        state
            .stream
            .as_ref()
            .unwrap()
            .cancel
            .store(true, Ordering::Relaxed);
        sender.send(Ok("late completion".into())).unwrap();
        assert!(state.poll().is_some());
        let draft = &state.drafts["chat"];
        assert!(draft.output.contains("partial text") && draft.output.contains("未完成"));
        assert!(draft.conversation.turns().is_empty());
        assert_eq!(draft.input, "question");
        assert!(!state.is_running() && state.stream.is_none());
    }
    #[test]
    fn chat_failure_keeps_draft_and_success_commits_once_in_original_tool() {
        let mut state = PluginState::new(
            std::env::temp_dir().join(format!("zi-chat-{}", uuid::Uuid::new_v4())),
        );
        let draft = state.drafts.entry("chat".into()).or_default();
        draft.conversation.bind("binding".into());
        draft.input = "question".into();
        for success in [false, true] {
            let draft = state.drafts.get_mut("chat").unwrap();
            draft.pending_chat = Some(("binding".into(), draft.input.clone()));
            let (sender, receiver) = mpsc::channel();
            state.running = Some(("chat".into(), receiver));
            state.select("other");
            sender
                .send(if success {
                    Ok("answer".into())
                } else {
                    Err("fixture error".into())
                })
                .unwrap();
            assert!(state.poll().is_some());
            let draft = &state.drafts["chat"];
            assert_eq!(draft.conversation.turns().len(), usize::from(success));
            assert_eq!(draft.input.is_empty(), success);
            assert!(draft.pending_chat.is_none());
            assert!(state.poll().is_none());
        }
        assert!(!state.drafts.contains_key("other"));
    }
    #[test]
    fn discovery_result_returns_to_source_tool_and_discards_changed_destination() {
        let root = std::env::temp_dir().join(format!("zi-discovery-ui-{}", uuid::Uuid::new_v4()));
        let mut state = PluginState::new(root);
        state.drafts.insert(
            "first".into(),
            Draft {
                endpoint: "http://localhost:1234/v1/chat/completions".into(),
                settings_key: "original".into(),
                ..Default::default()
            },
        );
        for changed in [false, true] {
            let (sender, receiver) = mpsc::channel();
            state.discovery_running = Some(DiscoveryJob {
                id: "first".into(),
                source: "http://localhost:1234/v1/chat/completions".into(),
                settings_key: "original".into(),
                cancel: Arc::new(AtomicBool::new(false)),
                receiver,
            });
            state.discovery_token = "synthetic-only".into();
            state.select("second");
            state.clear_token();
            assert!(state.discovery_token.is_empty());
            if changed {
                let draft = state.drafts.get_mut("first").unwrap();
                draft.endpoint = "http://localhost:4321/v1/chat/completions".into();
                draft.models.clear();
            }
            sender
                .send(Ok(crate::model_discovery::Report {
                    models: vec!["fixture".into()],
                    elapsed_ms: 1,
                }))
                .unwrap();
            assert!(state.is_running());
            assert!(state.poll().is_some());
            assert!(!state.is_running());
            assert_eq!(state.drafts["first"].models.is_empty(), changed);
            assert!(!state.drafts.contains_key("second"));
        }
    }
    #[test]
    fn plugin_tasks_expose_generic_status_and_stream_cancel_is_generation_bound() {
        use crate::tasks::Phase;

        let mut state = PluginState::new(
            std::env::temp_dir().join(format!("zi-plugin-task-{}", uuid::Uuid::new_v4())),
        );
        let id = "plugin:fixture/private-tool";
        let (sender, receiver) = mpsc::channel();
        state.execution_job.begin();
        let generation = state
            .execution_job
            .snapshot("plugin-execution", "插件工具请求", true)
            .unwrap()
            .generation;
        state.execution_instance = Some(id.into());
        state.running = Some((id.into(), receiver));
        state.stream = Some(StreamControl {
            cancel: Arc::new(AtomicBool::new(false)),
            partial: Arc::new(Mutex::new("partial output".into())),
        });

        let active = state.task_snapshots().pop().unwrap();
        assert_eq!(active.key, "plugin-execution");
        assert_eq!(active.instance.as_deref(), Some(id));
        assert_eq!(active.phase, Phase::Running);
        assert!(active.cancellable);
        assert!(active.summary.is_empty());
        assert!(!state.cancel_task(id, generation + 1));
        assert!(!state.cancel_task("plugin:other/tool", generation));
        assert!(state.cancel_task(id, generation));
        assert_eq!(state.task_snapshots()[0].phase, Phase::Cancelling);

        sender.send(Ok("private model output".into())).unwrap();
        assert!(state.poll().is_some());
        let finished = state.task_snapshots().pop().unwrap();
        assert_eq!(finished.phase, Phase::Cancelled);
        assert!(finished.summary.contains("部分内容留在插件工作台"));
        assert!(!finished.summary.contains("private model output"));
        assert!(state.drafts[id].output.contains("partial output"));
    }

    #[test]
    fn plugin_discovery_tasks_hide_errors_and_keep_status_generic() {
        use crate::tasks::Phase;

        let mut state = PluginState::new(
            std::env::temp_dir().join(format!("zi-plugin-discovery-task-{}", uuid::Uuid::new_v4())),
        );
        let id = "plugin:fixture/model-tool";
        let (sender, receiver) = mpsc::channel();
        state.discovery_job.begin();
        state.discovery_instance = Some(id.into());
        state.discovery_running = Some(DiscoveryJob {
            id: id.into(),
            source: "http://localhost/models".into(),
            settings_key: "fixture-settings".into(),
            cancel: Arc::new(AtomicBool::new(false)),
            receiver,
        });
        let active = state.task_snapshots().pop().unwrap();
        assert_eq!(active.key, "plugin-model-discovery");
        assert!(active.cancellable);
        sender
            .send(Err("private endpoint and credential".into()))
            .unwrap();
        assert!(state.poll().is_some());
        let finished = state.task_snapshots().pop().unwrap();
        assert_eq!(finished.phase, Phase::Failed);
        assert_eq!(finished.summary, "模型列表查询失败；详情留在插件连接设置中");
        assert!(!finished.summary.contains("private endpoint"));
        assert!(!finished.summary.contains("credential"));
    }

    #[test]
    fn plugin_discovery_cancel_is_generation_bound_and_discards_late_models() {
        use crate::tasks::Phase;

        let mut state = PluginState::new(std::env::temp_dir().join(format!(
            "zi-plugin-discovery-cancel-{}",
            uuid::Uuid::new_v4()
        )));
        let id = "plugin:fixture/model-tool";
        let (sender, receiver) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        state.discovery_job.begin();
        state.discovery_instance = Some(id.into());
        state.discovery_running = Some(DiscoveryJob {
            id: id.into(),
            source: "http://localhost/models".into(),
            settings_key: "fixture-settings".into(),
            cancel: Arc::clone(&cancel),
            receiver,
        });
        let generation = state.task_snapshots().pop().unwrap().generation;
        assert!(!state.cancel_task(id, generation.wrapping_add(1)));
        assert!(state.cancel_task(id, generation));
        assert!(cancel.load(Ordering::Relaxed));
        sender
            .send(Ok(crate::model_discovery::Report {
                models: vec!["late-model".into()],
                elapsed_ms: 1,
            }))
            .unwrap();
        assert!(state.poll().is_some());
        let row = state.task_snapshots().pop().unwrap();
        assert_eq!(row.phase, Phase::Cancelled);
        assert!(!row.cancellable);
        assert!(state.drafts[id].models.is_empty());
    }

    #[test]
    fn plugin_http_request_cancel_is_generation_bound_and_uses_generic_summary() {
        use crate::tasks::Phase;

        let mut state = PluginState::new(
            std::env::temp_dir().join(format!("zi-plugin-http-cancel-{}", uuid::Uuid::new_v4())),
        );
        let id = "plugin:fixture/http-tool";
        let (sender, receiver) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        state.execution_job.begin();
        state.execution_instance = Some(id.into());
        state.request_cancel = Some(Arc::clone(&cancel));
        state.running = Some((id.into(), receiver));
        let generation = state.task_snapshots().pop().unwrap().generation;
        assert!(!state.cancel_task(id, generation.wrapping_add(1)));
        assert!(state.cancel_task(id, generation));
        sender
            .send(Err("sensitive endpoint, arguments and token".into()))
            .unwrap();
        assert!(state.poll().is_some());
        let row = state.task_snapshots().pop().unwrap();
        assert_eq!(row.phase, Phase::Cancelled);
        assert!(!row.cancellable);
        assert!(row.summary.contains("服务端可能仍在处理"));
        assert!(!row.summary.contains("sensitive"));
    }
    #[cfg(windows)]
    #[test]
    #[ignore = "explicit isolated Windows vault and localhost HTTP integration"]
    fn saved_credential_reaches_only_bound_tool_and_is_redacted_from_response() {
        use std::{
            io::{Read, Write},
            net::TcpListener,
            time::Duration,
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/test", listener.local_addr().unwrap());
        let id = format!("plugin:test-{}/request", uuid::Uuid::new_v4());
        let target = crate::credentials::Target::plugin(&id, "GET", &url).unwrap();
        struct Cleanup(crate::credentials::Target);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = crate::credentials::delete(&self.0);
            }
        }
        let cleanup = Cleanup(target);
        crate::credentials::save(&cleanup.0, "synthetic-fixture-token").unwrap();
        let tool = plugins::PluginTool {
            id: "request".into(),
            version: None,
            version_history: Vec::new(),
            name: "Fixture".into(),
            description: String::new(),
            category: "AI 与模型".into(),
            keywords: vec![],
            aliases: vec![],
            sample: String::new(),
            model: String::new(),
            adapter: Adapter::Http {
                url: url.clone(),
                method: "GET".into(),
                body: serde_json::Value::Null,
                response_pointer: Some("/output".into()),
            },
        };
        // A different tool identity must fail before sending any request.
        assert!(
            execute_with_credentials(
                &tool,
                &format!("{id}-other"),
                "",
                "",
                String::new(),
                true,
                None
            )
            .is_err()
        );
        let server = std::thread::spawn(move || {
            listener.set_nonblocking(true).unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            let mut stream = loop {
                match listener.accept() {
                    Ok((s, _)) => break s,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(
                            std::time::Instant::now() < deadline,
                            "fixture request timed out"
                        );
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => panic!("fixture accept failed"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 1024];
            while !request.windows(4).any(|w| w == b"\r\n\r\n") && request.len() < 8192 {
                let n = stream.read(&mut buffer).unwrap();
                if n == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..n]);
            }
            let authorized = String::from_utf8_lossy(&request)
                .to_ascii_lowercase()
                .contains("authorization: bearer synthetic-fixture-token");
            let body = r#"{"output":"synthetic-fixture-token"}"#;
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
            authorized
        });
        let result =
            execute_with_credentials(&tool, &id, "", "", String::new(), true, None).unwrap();
        assert!(server.join().unwrap());
        assert_eq!(result, "[REDACTED]");
    }
    #[test]
    fn background_result_keeps_own_draft_and_clears_busy() {
        let root = std::env::temp_dir().join(format!("zi-plugin-ui-{}", uuid::Uuid::new_v4()));
        let mut state = PluginState::new(root);
        let (tx, rx) = mpsc::channel();
        state.running = Some(("first".into(), rx));
        state.select("second");
        assert!(state.poll().is_none());
        tx.send(Ok("done".into())).unwrap();
        assert!(state.poll().is_some());
        assert!(!state.is_running());
        assert_eq!(state.drafts["first"].output, "done");
        assert_eq!(state.selected.as_deref(), Some("second"));
        state.token = "test-only".into();
        state.use_saved = true;
        state.select("first");
        assert!(state.token.is_empty());
        assert!(!state.use_saved);
        let (tx, rx) = mpsc::channel();
        state.running = Some(("first".into(), rx));
        drop(tx);
        assert!(state.poll().unwrap().contains("失败"));
        assert!(!state.is_running());
    }
}
