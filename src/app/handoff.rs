use super::*;
#[cfg(feature = "ui-preview")]
mod date_preview;
mod discovery;
mod keyboard;
#[cfg(feature = "ui-preview")]
mod keyboard_preview;
#[cfg(feature = "ui-preview")]
mod numeric_preview;
#[cfg(feature = "ui-preview")]
mod plot_preview;
#[cfg(feature = "ui-preview")]
mod table_statistics_preview;
use crate::calculator::exchange::{MatrixSlot, NumericTable, Representation};
use crate::text_flow::material::Material;

fn versioned_handoff_source(source: String, page: Page) -> String {
    let tool_id = match page {
        Page::Agent => Some("agent-runner"),
        Page::Mcp => Some("mcp-inspector"),
        Page::Plugins => Some("plugins"),
        _ => None,
    };
    let Some(tool_id) = tool_id else {
        return source;
    };
    let Ok(provenance) = crate::tool_provenance::current_for_tool(tool_id) else {
        return source;
    };
    format!(
        "{source} · {} v{}",
        provenance.tool_id, provenance.tool_version
    )
}

fn calculator_version() -> &'static str {
    catalog()
        .iter()
        .find(|entry| entry.id == "advanced-calculator")
        .and_then(|entry| entry.version.as_deref())
        .unwrap_or("未声明")
}

fn reset_scroll_area(ui: &egui::Ui, salt: &str) {
    // Reset pending animations and touch momentum as well as the offset. A new
    // handoff/filter must not resume the previous list's motion on later frames.
    egui::scroll_area::State::default().store(ui.ctx(), ui.make_persistent_id(egui::Id::new(salt)));
}

fn target_button(
    ui: &mut egui::Ui,
    transfer: &mut Transfer,
    target: Target,
    reveal: Option<Target>,
) -> egui::Response {
    let response = ui.selectable_value(&mut transfer.target, target, target.label());
    if response.changed() {
        transfer.error.clear();
    }
    if response.clicked() {
        transfer.navigation.cursor = Some(target);
    }
    if reveal == Some(target) {
        response.scroll_to_me(Some(egui::Align::Center));
    }
    #[cfg(feature = "ui-preview")]
    ui.ctx().data_mut(|data| {
        data.insert_temp(
            egui::Id::new(("handoff-target-proof", target.label())),
            response.rect,
        );
        if target == transfer.target {
            data.insert_temp(
                egui::Id::new("handoff-selected-row-proof"),
                (response.rect, ui.clip_rect()),
            );
        }
    });
    response
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Tool(ToolKind),
    Diagnostic(crate::framework::Tool),
    Csv,
    Tsv,
    JsonData,
    TextFlow,
    Before,
    After,
    Memo,
    Event,
    ArtifactHub,
    KnowledgeCapture,
    Calculator,
    Agent,
    FileEncoding,
}
impl Target {
    fn label(self) -> &'static str {
        match self {
            Self::Tool(tool) => tool.label(),
            Self::Diagnostic(tool) => tool.label(),
            Self::Csv => "数据工作台 · CSV",
            Self::Tsv => "数据工作台 · TSV",
            Self::JsonData => "数据工作台 · JSON 对象数组",
            Self::TextFlow => "操作流程 · 文本工具链（新实例）",
            Self::Before => "文本对比 · 左侧原文",
            Self::After => "文本对比 · 右侧新文",
            Self::Memo => "备忘录 · 新建草稿",
            Self::Event => "万年历 / 日程 · 新建草稿",
            Self::ArtifactHub => "本机资料库 · 保存为临时草稿",
            Self::KnowledgeCapture => "网页与对话收集 · 保存到知识源",
            Self::Calculator => "计算器 · 数值表格填入矩阵",
            Self::Agent => "本机 Agent · 添加接力资料",
            Self::FileEncoding => "文件编码检查 · 原文件引用",
        }
    }
    fn all() -> Vec<Self> {
        ToolKind::ALL
            .into_iter()
            .filter(|kind| !matches!(kind, ToolKind::Uuid | ToolKind::Random))
            .map(Self::Tool)
            .chain([
                Self::Csv,
                Self::Tsv,
                Self::JsonData,
                Self::TextFlow,
                Self::Before,
                Self::After,
                Self::Memo,
                Self::Event,
                Self::ArtifactHub,
                Self::KnowledgeCapture,
                Self::Calculator,
                Self::Agent,
                Self::FileEncoding,
            ])
            .chain(
                crate::framework::Tool::ALL
                    .into_iter()
                    .filter(|tool| tool.accepts_text_handoff())
                    .map(Self::Diagnostic),
            )
            .collect()
    }
}

pub(super) struct Transfer {
    matrix_slot: MatrixSlot,
    allow_matrix_replace: bool,
    numeric: Option<NumericTable>,
    table_numeric: Option<NumericTable>,
    material: Option<Material>,
    event_date: Option<chrono::NaiveDate>,
    representation: Representation,
    numeric_rendered: Option<(Representation, Target)>,
    source: String,
    origin_service: Option<String>,
    source_provenance: Option<crate::tool_provenance::ToolProvenance>,
    source_reference: Option<String>,
    text: String,
    preview: String,
    query: String,
    focus_query: bool,
    navigation: keyboard::Navigation,
    target: Target,
    error: String,
    new_data_instance: bool,
    category: String,
    scope: discovery::Scope,
    access: Option<discovery::Access>,
    recommendations: Vec<discovery::Recommendation>,
    matches_key: Option<(String, String, discovery::Scope)>,
    matches: Vec<Target>,
    reset_body_scroll: bool,
    reset_target_scroll: bool,
    await_new_wheel: bool,
    #[cfg(feature = "ui-preview")]
    preview_rects: [Option<egui::Rect>; 2],
    #[cfg(feature = "ui-preview")]
    numeric_mode_rects: [Option<egui::Rect>; 3],
    #[cfg(feature = "ui-preview")]
    matrix_rects: [Option<egui::Rect>; 3],
    #[cfg(feature = "ui-preview")]
    preview_recommendation_rect: Option<egui::Rect>,
    #[cfg(feature = "ui-preview")]
    preview_log_target: Option<egui::Rect>,
}
impl Transfer {
    pub(super) fn new(source: String, text: &str) -> anyhow::Result<Self> {
        anyhow::ensure!(!text.is_empty(), "当前没有可发送的结果");
        anyhow::ensure!(
            text.len() <= 2 * 1024 * 1024,
            "结果超过 2 MiB，请先缩小范围"
        );
        Ok(Self {
            matrix_slot: MatrixSlot::A,
            allow_matrix_replace: false,
            numeric: None,
            table_numeric: None,
            material: None,
            event_date: None,
            representation: Representation::Typed,
            numeric_rendered: None,
            source,
            origin_service: None,
            source_provenance: None,
            source_reference: None,
            text: text.into(),
            preview: text.chars().take(1200).collect(),
            query: String::new(),
            focus_query: true,
            navigation: Default::default(),
            target: Target::Tool(ToolKind::Json),
            error: String::new(),
            new_data_instance: true,
            category: String::new(),
            scope: discovery::Scope::All,
            access: None,
            recommendations: discovery::recommendations(text),
            matches_key: None,
            matches: Vec::new(),
            reset_body_scroll: true,
            reset_target_scroll: true,
            await_new_wheel: true,
            #[cfg(feature = "ui-preview")]
            preview_rects: [None; 2],
            #[cfg(feature = "ui-preview")]
            numeric_mode_rects: [None; 3],
            #[cfg(feature = "ui-preview")]
            matrix_rects: [None; 3],
            #[cfg(feature = "ui-preview")]
            preview_recommendation_rect: None,
            #[cfg(feature = "ui-preview")]
            preview_log_target: None,
        })
    }
    fn dated(source: String, date: chrono::NaiveDate, text: &str) -> anyhow::Result<Self> {
        use chrono::Datelike;
        anyhow::ensure!((1..=9999).contains(&date.year()), "日期快照支持0001–9999年");
        let mut transfer = Self::new(source, text)?;
        transfer.event_date = Some(date);
        transfer.target = Target::Event;
        Ok(transfer)
    }
    fn numeric(source: String, table: NumericTable) -> anyhow::Result<Self> {
        let text = table
            .json(Representation::Typed)
            .map_err(anyhow::Error::msg)?;
        let mut transfer = Self::new(source, &text)?;
        transfer.numeric = Some(table);
        transfer.target = Target::JsonData;
        Ok(transfer)
    }
    pub(super) fn material(source: String, material: Material) -> anyhow::Result<Self> {
        let is_image = matches!(&material, Material::Image(_));
        anyhow::ensure!(
            matches!(
                material.kind(),
                crate::text_flow::material::Kind::Table | crate::text_flow::material::Kind::Image
            ),
            "此接力入口当前仅接收结构化表格或内存图片材料"
        );
        let text = match &material {
            Material::Table(_) => material.render()?,
            Material::Image(_) => material.summary(),
            _ => unreachable!("material kind was checked above"),
        };
        anyhow::ensure!(
            text.len() <= 2 * 1024 * 1024,
            "材料摘要超过 2 MiB，无法接力"
        );
        let mut transfer = Self::new(source, &text)?;
        transfer.preview = material.preview()?;
        transfer.table_numeric = match &material {
            Material::Table(dataset) => dataset.numeric_snapshot().ok(),
            _ => None,
        };
        if transfer.table_numeric.is_some() {
            transfer.recommendations.insert(
                0,
                discovery::Recommendation {
                    target: Target::Calculator,
                    reason: "完整数值表格可直接填入计算器A/B矩阵；预览映射且不自动计算",
                },
            );
        } else if is_image {
            transfer.recommendations.insert(
                0,
                discovery::Recommendation {
                    target: Target::TextFlow,
                    reason: "将有界内存图片接入独立文本流程；运行 image.info 后才生成报告",
                },
            );
        }
        transfer.material = Some(material);
        transfer.target = if matches!(transfer.material.as_ref(), Some(Material::Image(_))) {
            Target::TextFlow
        } else {
            Target::JsonData
        };
        Ok(transfer)
    }
    fn artifact_snapshot(
        source: String,
        text: String,
        table: Option<crate::workbench::Dataset>,
    ) -> anyhow::Result<Self> {
        if let Some(dataset) = table {
            Self::material(source, Material::Table(dataset))
        } else {
            Self::new(source, &text)
        }
    }
    pub(super) fn set_source_provenance(
        &mut self,
        provenance: Option<crate::tool_provenance::ToolProvenance>,
    ) {
        self.source_provenance = provenance;
    }
    pub(super) fn file(
        source: String,
        file: crate::material_files::FileMaterial,
    ) -> anyhow::Result<Self> {
        let material = Material::File(file);
        let preview = material.preview()?;
        let mut transfer = Self::new(source, &preview)?;
        transfer.preview = preview;
        transfer.material = Some(material);
        transfer.target = Target::FileEncoding;
        Ok(transfer)
    }
    fn refresh_numeric(&mut self) {
        let table = self.numeric.as_ref().or_else(|| {
            (self.target == Target::Calculator)
                .then_some(self.table_numeric.as_ref())
                .flatten()
        });
        let Some(table) = table else {
            if matches!(self.material.as_ref(), Some(Material::Table(_)))
                && matches!(self.numeric_rendered, Some((_, Target::Calculator)))
            {
                if let Some(material) = &self.material {
                    self.text = material.render().unwrap_or_default();
                    self.preview = material.preview().unwrap_or_default();
                }
                self.numeric_rendered = None;
            }
            return;
        };
        let key = (self.representation, self.target);
        if self.numeric_rendered == Some(key) {
            return;
        }
        let result = match self.target {
            Target::Csv => table.delimited(self.representation, b','),
            Target::Tsv => table.delimited(self.representation, b'\t'),
            _ => table.json(self.representation),
        };
        match result {
            Ok(text) => {
                self.text = text;
                self.preview = self.text.chars().take(1200).collect();
                self.error.clear();
                self.numeric_rendered = Some(key);
            }
            Err(error) => {
                self.preview = format!("无法生成此格式：{error}");
                self.text.clear();
                self.error = error;
                self.numeric_rendered = Some(key);
            }
        }
    }
    fn compatible(&self) -> bool {
        if !self.accepts_target(self.target) {
            return false;
        }
        if self.event_date.is_some() && self.target == Target::Calculator {
            return false;
        }
        if self.target == Target::KnowledgeCapture
            && self.text.len() > crate::knowledge_capture::MAX_HANDOFF_BODY
        {
            return false;
        }
        !(self.numeric.is_some()
            && self.representation == Representation::Typed
            && matches!(self.target, Target::Csv | Target::Tsv))
    }
    fn accepts_target(&self, target: Target) -> bool {
        match self.material.as_ref() {
            Some(Material::Table(_)) => {
                matches!(target, Target::JsonData | Target::ArtifactHub)
                    || target == Target::Calculator && self.table_numeric.is_some()
            }
            Some(Material::File(file)) => {
                target == Target::TextFlow
                    || target == Target::FileEncoding
                        && file.bytes() <= crate::file_encoding::MAX_INPUT as u64
            }
            Some(Material::Image(_)) => {
                matches!(target, Target::TextFlow | Target::ArtifactHub)
            }
            None => true,
            Some(Material::Text(_)) => false,
        }
    }
    fn artifact_kind(&self) -> crate::artifact_hub::Kind {
        if matches!(self.material.as_ref(), Some(Material::Table(_))) {
            crate::artifact_hub::Kind::Table
        } else {
            crate::artifact_hub::Kind::Text
        }
    }
    fn apply_material(
        &self,
        data: &mut crate::workbench::sessions::Workspace,
    ) -> anyhow::Result<(Page, Option<ToolKind>)> {
        anyhow::ensure!(
            self.target == Target::JsonData,
            "结构化表格只能发送到数据工作台"
        );
        match &self.material {
            Some(Material::Table(dataset)) => {
                data.import_table(dataset.clone(), &self.source)?;
                Ok((Page::Data, None))
            }
            Some(Material::Image(_)) => anyhow::bail!("图片材料需由文本流程或本机资料库接收"),
            Some(Material::File(_)) => anyhow::bail!("文件引用需由文件编码检查接收"),
            _ => anyhow::bail!("没有可接收的结构化材料"),
        }
    }
    fn apply_image(
        &self,
        data: &mut crate::workbench::sessions::Workspace,
    ) -> anyhow::Result<(Page, Option<ToolKind>)> {
        anyhow::ensure!(self.target == Target::TextFlow, "图片材料只能接入文本流程");
        match &self.material {
            Some(Material::Image(image)) => {
                data.import_text_flow_image(image.clone(), "接力图片流程")?;
                Ok((Page::Data, None))
            }
            _ => anyhow::bail!("没有可接收的图片材料"),
        }
    }
    fn apply_file(
        &self,
        file_encoding: &mut crate::file_encoding::State,
    ) -> anyhow::Result<(Page, Option<ToolKind>)> {
        anyhow::ensure!(
            self.target == Target::FileEncoding,
            "文件引用只能发送到文件编码检查"
        );
        match &self.material {
            Some(Material::File(file)) => {
                file_encoding.receive_material(file.clone())?;
                Ok((Page::FileEncoding, None))
            }
            _ => anyhow::bail!("没有可接收的文件引用"),
        }
    }
    #[cfg(feature = "ui-preview")]
    pub(super) fn preview_typed_table_ready(&self) -> bool {
        self.material.is_some() && self.compatible() && self.preview.contains("Zi Tools")
    }
    #[cfg(feature = "ui-preview")]
    pub(super) fn preview_file_ready(&self) -> bool {
        matches!(self.material.as_ref(), Some(Material::File(_)))
            && self.target == Target::FileEncoding
            && self.compatible()
            && self.preview.contains("source.txt")
            && !self.preview.contains("AppData")
    }
    #[cfg(feature = "ui-preview")]
    pub(super) fn preview_apply_position(&self) -> egui::Pos2 {
        self.preview_rects[1]
            .expect("shared handoff confirmation rendered")
            .center()
    }
    fn discard_old_wheel(&mut self, ctx: &egui::Context, reset: bool) {
        // ScrollArea::State does not own egui's pending wheel smoothing. After
        // a reset, consume the remaining deltas until a new real wheel event.
        // Scrollbar dragging and keyboard/programmatic scrolling stay available.
        if reset {
            self.await_new_wheel = true;
        } else if ctx.input(|input| input.raw_scroll_delta != egui::Vec2::ZERO) {
            self.await_new_wheel = false;
        }
        if self.await_new_wheel {
            ctx.input_mut(|input| input.smooth_scroll_delta = egui::Vec2::ZERO);
        }
    }
    fn matching_targets(&mut self) -> Vec<Target> {
        let key = (
            registry::normalized(&self.query),
            self.category.clone(),
            self.scope,
        );
        if self.matches_key.as_ref() != Some(&key) {
            self.navigation.cursor = None;
            self.matches = discovery::search(&key.0, &key.1);
            if let Some(access) = &self.access {
                access.filter(&mut self.matches, self.scope, !key.0.is_empty());
            }
            self.matches_key = Some(key);
            self.reset_target_scroll = true;
        }
        self.matches
            .iter()
            .copied()
            .filter(|target| {
                !(!self.accepts_target(*target)
                    || self.event_date.is_some() && *target == Target::Calculator
                    || self.numeric.is_some()
                        && self.representation == Representation::Typed
                        && matches!(target, Target::Csv | Target::Tsv))
            })
            .collect()
    }
    fn apply(
        &self,
        tools: &mut ToolState,
        data: &mut DataState,
        diff: &mut DiffState,
        artifacts: &mut crate::artifact_hub::State,
    ) -> anyhow::Result<(Page, Option<ToolKind>)> {
        match self.target {
            Target::ArtifactHub => {
                artifacts.receive_text_with_reference(
                    &self.text,
                    self.artifact_kind(),
                    self.source_provenance.as_ref(),
                    self.source_reference.as_deref(),
                )?;
                Ok((Page::Artifacts, None))
            }
            Target::Memo
            | Target::Event
            | Target::Calculator
            | Target::Agent
            | Target::KnowledgeCapture
            | Target::Diagnostic(_)
            | Target::TextFlow
            | Target::FileEncoding => {
                anyhow::bail!("此接力目标需通过类型化材料入口接收")
            }
            Target::Tool(kind) => {
                tools.select(kind);
                tools.input.clone_from(&self.text);
                tools.clear_result();
                tools.qr_image = None;
                Ok((
                    if kind.is_encoding() {
                        Page::EncodingTools
                    } else {
                        Page::SmallTools
                    },
                    Some(kind),
                ))
            }
            Target::Csv | Target::Tsv | Target::JsonData => {
                data.import_text(
                    self.text.clone(),
                    if self.target == Target::JsonData {
                        crate::workbench::DataFormat::Json
                    } else {
                        crate::workbench::DataFormat::Csv
                    },
                    self.target == Target::Tsv,
                )?;
                Ok((Page::Data, None))
            }
            Target::Before | Target::After => {
                if self.target == Target::Before {
                    diff.diff_before.clone_from(&self.text);
                } else {
                    diff.diff_after.clone_from(&self.text);
                }
                diff.diff_output.clear();
                diff.message.clear();
                Ok((Page::Diff, None))
            }
        }
    }
}

impl DevToolsApp {
    #[cfg(feature = "ui-preview")]
    pub fn preview_text_flow_image_relay_prepare(&mut self, ctx: &egui::Context, light: bool) {
        self.set_theme(ctx, if light { Theme::Light } else { Theme::Dark });
        self.startup_warning = None;
        self.page = Page::Images;
        self.images = Default::default();
        self.images.preview_relay_fixture(ctx);
        self.open_startup_tool("image-tools");
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_text_flow_image_relay_ready(&self) -> bool {
        self.images.preview_text_flow_relay_ready()
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_text_flow_image_relay_check(&self, phase: u8) {
        match phase {
            0 => {
                assert_eq!(self.page, Page::Images);
                self.images.preview_text_flow_relay_check();
                assert_eq!(self.data_state.instances.len(), 1);
            }
            1 => {
                assert_eq!(self.page, Page::Data);
                assert_eq!(self.data_state.instances.len(), 2);
                self.data_state.preview_text_flow_image_input_check();
                self.images.preview_text_flow_relay_source_retained();
            }
            _ => panic!("image-to-text-flow relay phase"),
        }
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_image_report_prepare(&mut self, ctx: &egui::Context, light: bool) {
        self.set_theme(ctx, if light { Theme::Light } else { Theme::Dark });
        self.startup_warning = None;
        self.page = Page::Images;
        self.images = Default::default();
        self.images.preview_fixture(ctx);
        self.open_startup_tool("image-tools");
        self.tool_state.select(ToolKind::Json);
        self.tool_state.input = "existing-target-draft".into();
        self.tool_state.clear_result();
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_batch_report_prepare(&mut self, ctx: &egui::Context, light: bool) {
        self.preview_image_report_prepare(ctx, light);
        self.images.preview_batch_report_fixture();
        self.open_startup_tool("image-batch");
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_metadata_report_prepare(&mut self, ctx: &egui::Context, light: bool) {
        self.preview_image_report_prepare(ctx, light);
        self.images.preview_metadata_report_fixture(ctx);
        self.open_startup_tool("image-metadata");
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_metadata_relay_prepare(
        &mut self,
        ctx: &egui::Context,
        light: bool,
    ) -> std::sync::Arc<Vec<u8>> {
        self.set_theme(ctx, if light { Theme::Light } else { Theme::Dark });
        self.startup_warning = None;
        self.page = Page::Images;
        self.images = Default::default();
        let source = self.images.preview_metadata_relay_fixture(ctx);
        self.open_startup_tool("image-metadata");
        source
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_metadata_relay_ready(&self) -> bool {
        self.images.preview_metadata_relay_ready()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_metadata_relay_status(&self) -> String {
        self.images.preview_metadata_relay_status()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_metadata_relay_check(&self, phase: u8, source: &std::sync::Arc<Vec<u8>>) {
        self.images.preview_metadata_relay_check(phase, source);
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_metadata_relay_reopen(&mut self) {
        self.open_startup_tool("image-metadata");
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_batch_relay_prepare(
        &mut self,
        ctx: &egui::Context,
        light: bool,
        root: &std::path::Path,
    ) -> std::sync::Arc<Vec<u8>> {
        self.set_theme(ctx, if light { Theme::Light } else { Theme::Dark });
        self.startup_warning = None;
        self.page = Page::Images;
        self.images = Default::default();
        let source = self.images.preview_batch_relay_fixture(ctx, root);
        self.open_startup_tool("image-batch");
        source
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_batch_relay_check(&self, phase: u8, source: &std::sync::Arc<Vec<u8>>) {
        self.images.preview_batch_relay_check(phase, source);
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_batch_relay_reopen(&mut self) {
        self.open_startup_tool("image-batch");
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_image_workflow_prepare(&mut self, ctx: &egui::Context, light: bool) {
        self.set_theme(ctx, if light { Theme::Light } else { Theme::Dark });
        self.startup_warning = None;
        self.page = Page::Images;
        self.images = Default::default();
        self.images.preview_image_workflow_fixture();
        self.open_startup_tool("image-workflow");
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_saved_image_workflow_prepare(
        &mut self,
        ctx: &egui::Context,
        light: bool,
        folder: &std::path::Path,
    ) {
        self.preview_image_workflow_prepare(ctx, light);
        let path = folder.join("saved-tutorial.json");
        let bytes = self.images.preview_image_workflow_definition();
        std::fs::write(&path, &bytes).unwrap();
        let entry = crate::workflow_document::Document::parse(&bytes)
            .unwrap()
            .metadata(&path);
        self.preferences.workflow_favorites.push(entry.clone());
        self.open_search_choice(super::launcher::Choice::SavedWorkflow(entry));
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_saved_image_workflow_pending(&self) -> bool {
        self.images.preview_image_workflow_import_pending()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_saved_image_workflow_check(&self) {
        assert_eq!(self.page, Page::Images);
        assert!(!self.images.preview_image_workflow_import_pending());
        assert!(
            self.preferences
                .workflow_recent
                .iter()
                .any(|v| v.name == "图片流程 · saved-tutorial")
        );
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_image_instances_check(&self, phase: u8) {
        self.images.preview_image_instances_check(phase);
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_image_workflow_ready(&self) -> bool {
        self.images.preview_image_workflow_ready()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_image_workflow_check(&self, phase: u8) {
        self.images.preview_image_workflow_check(phase);
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_image_report_check(&self, phase: u8) {
        let text = self.images.preview_image_report_assert();
        match phase {
            0 => {
                assert!(self.handoff.is_none());
                assert_eq!(self.page, Page::Images);
                assert_eq!(self.handoff_source().unwrap().1, text);
            }
            1 => {
                assert_eq!(self.handoff.as_ref().unwrap().text, text);
                assert_eq!(self.tool_state.input, "existing-target-draft");
            }
            2 => {
                assert!(self.handoff.is_none());
                assert_eq!(self.tool_state.input, "existing-target-draft");
            }
            3 => {
                assert!(self.handoff.is_none());
                assert_eq!(self.tool_state.selected, ToolKind::Json);
                assert_eq!(self.tool_state.input, text);
                assert!(!self.tool_state.has_result());
            }
            4 => {
                assert!(self.handoff.is_none());
                assert_eq!(self.page, Page::Notes);
                assert!(self.planner.has_unsaved());
                assert_eq!(self.planner.transfer_text().unwrap().1, text);
            }
            _ => panic!("image report phase"),
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_image_report_memo_prepare(&mut self) {
        self.planner.preview(false, false);
        let text = self.images.preview_image_report_assert();
        let mut transfer = Transfer::new("图片信息报告（冻结快照）".into(), &text).unwrap();
        transfer.target = Target::Memo;
        self.handoff = Some(transfer);
        self.page = Page::Images;
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_file_report_dispatch(&mut self) {
        let report = self.file_state.report().to_owned();
        let (source, text) = self.handoff_source().unwrap();
        let mut transfer = Transfer::new(source, text).unwrap();
        transfer.target = Target::Tool(ToolKind::Json);
        self.handoff = Some(transfer);
        self.apply_handoff();
        assert_eq!(self.tool_state.input, report);
        assert!(!self.tool_state.has_result());
        assert_eq!(self.file_state.report(), report);
        self.page = Page::Files;
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_file_flow_handoff_prepare(
        &mut self,
        ctx: &egui::Context,
        light: bool,
        path: &std::path::Path,
    ) {
        self.set_theme(ctx, if light { Theme::Light } else { Theme::Dark });
        self.startup_warning = None;
        std::fs::write(path, b"flow handoff fixture").unwrap();
        let file = crate::material_files::FileMaterial::selected(path, usize::MAX).unwrap();
        let mut transfer = Transfer::file("测试文件引用".into(), file).unwrap();
        transfer.target = Target::TextFlow;
        self.handoff = Some(transfer);
        self.page = Page::Files;
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_file_flow_handoff_check(&self, phase: u8, path: &std::path::Path) {
        match phase {
            0 => {
                let transfer = self.handoff.as_ref().unwrap();
                assert_eq!(transfer.target, Target::TextFlow);
                assert!(transfer.compatible());
                assert!(
                    !transfer
                        .preview
                        .contains(path.parent().unwrap().to_string_lossy().as_ref())
                );
                assert_eq!(self.data_state.instances.len(), 1);
            }
            1 => {
                assert!(self.handoff.is_none());
                assert_eq!(self.data_state.instances.len(), 2);
                assert_eq!(self.data_state.active_tool_id(), "text-flow");
                assert!(self.data_state.has_work());
                assert!(!self.data_state.busy());
                assert_eq!(
                    self.toast.as_ref().map(|(message, _)| message.as_str()),
                    Some("文件引用已加入新流程；尚未读取")
                );
                let snapshot = self.data_state.snapshot().unwrap();
                let text = String::from_utf8(snapshot).unwrap();
                assert!(!text.contains(&path.to_string_lossy().to_string()));
                assert!(!text.contains("flow handoff fixture"));
                assert_eq!(std::fs::read(path).unwrap(), b"flow handoff fixture");
            }
            _ => panic!("file flow handoff phase"),
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_file_flow_output_ready(&self) -> bool {
        self.data_state.preview_file_flow_output_ready()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_file_flow_report_selected(&self) -> bool {
        self.data_state.preview_file_flow_report_selected()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_file_flow_output_choice_position(&self, path: &std::path::Path) -> egui::Pos2 {
        self.data_state
            .preview_file_flow_output_choice_position(path)
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_file_flow_output_selected(&self, path: &std::path::Path) -> bool {
        self.data_state.preview_file_flow_output_selected(path)
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_file_flow_output_position(&self) -> egui::Pos2 {
        self.data_state.preview_file_flow_output_position()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_file_flow_output_handoff_check(&self, phase: u8, path: &std::path::Path) {
        match phase {
            0 => {
                let transfer = self.handoff.as_ref().expect("file output handoff dialog");
                assert_eq!(self.page, Page::Data);
                assert_eq!(transfer.target, Target::FileEncoding);
                assert!(transfer.compatible());
                assert!(
                    !transfer
                        .preview
                        .contains(&path.to_string_lossy().to_string())
                );
                assert_eq!(
                    transfer.preview,
                    "文件引用 · flow-handoff.txt · 20字节；内容尚未读取；预览不读取文件内容"
                );
            }
            1 => {
                assert!(self.handoff.is_none());
                assert_eq!(self.page, Page::FileEncoding);
                self.file_encoding.preview_material_check(false);
                assert_eq!(std::fs::read(path).unwrap(), b"flow handoff fixture");
            }
            _ => panic!("file output handoff phase"),
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_handoff_result_prepare(&mut self, ctx: &egui::Context, light: bool) {
        self.set_theme(ctx, if light { Theme::Light } else { Theme::Dark });
        self.startup_warning = None;
        self.tool_state.select(ToolKind::Base64);
        self.tool_state.input.clear();
        let result = self.tool_state.run(0);
        assert!(self.tool_state.apply_result(result));
        assert!(self.tool_state.has_result() && self.tool_state.output.is_empty());
        self.tool_state.select(ToolKind::Json);
        self.tool_state.input = r#"{"source":true}"#.into();
        let result = self.tool_state.run(0);
        assert!(self.tool_state.apply_result(result));
        let mut transfer = Transfer::new("JSON source".into(), &self.tool_state.output).unwrap();
        transfer.target = Target::Tool(ToolKind::Base64);
        self.handoff = Some(transfer);
        self.apply_handoff();
        assert!(self.handoff.is_none());
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_handoff_result_copy_position(&self) -> egui::Pos2 {
        self.preview_text_copy
            .expect("rendered copy button")
            .0
            .center()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_handoff_result_check(&mut self, ran: bool) {
        assert_eq!(self.tool_state.selected, ToolKind::Base64);
        assert_eq!(self.tool_state.input, "{\n  \"source\": true\n}");
        assert_eq!(self.tool_state.has_result(), ran);
        assert_eq!(self.preview_text_copy.expect("rendered copy button").1, ran);
        if ran {
            let preview = self.tool_state.run(0).unwrap();
            assert_eq!(self.tool_state.output, preview);
        } else {
            assert!(self.tool_state.output.is_empty());
        }
        self.tool_state.select(ToolKind::Json);
        assert_eq!(self.tool_state.input, r#"{"source":true}"#);
        assert!(self.tool_state.has_result());
        assert_eq!(self.tool_state.output, "{\n  \"source\": true\n}");
        self.tool_state.select(ToolKind::Base64);
    }
    pub(super) fn send_service_logs(&mut self, service_id: &str, filtered: bool) {
        if self.log_inflight || self.log_error || self.handoff.is_some() {
            return;
        }
        self.log_lines.update(&self.log_text, &self.log_filter);
        let text = if filtered {
            self.log_lines.copy_matches(&self.log_text)
        } else {
            self.log_text.clone()
        };
        let source = format!(
            "服务日志 · {service_id} · {} · 末尾有界快照",
            if filtered {
                "筛选结果"
            } else {
                "全部预览"
            }
        );
        match Transfer::new(source, &text) {
            Ok(mut transfer) => {
                transfer.origin_service = Some(service_id.into());
                self.handoff = Some(transfer);
                self.log_view = None;
            }
            Err(error) => {
                self.toast = Some((error.to_string(), Instant::now()));
            }
        }
    }
    fn cancel_handoff(&mut self) {
        let origin = self
            .handoff
            .take()
            .and_then(|transfer| transfer.origin_service);
        if let Some(id) = origin.filter(|id| self.manager.service_spec(id).is_ok()) {
            self.log_view = Some(id);
            self.last_logs = Instant::now();
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_log_transfer_scene(&mut self) {
        self.page = Page::Services;
        self.startup_warning = None;
        self.log_inflight = false;
        self.log_error = false;
        self.handoff = None;
        self.log_text = crate::framework::Tool::Gc.sample().into();
        self.log_lines.invalidate();
        self.log_filter = "gc".into();
        self.send_service_logs("demo", true);
        let transfer = self.handoff.as_mut().unwrap();
        transfer.target = Target::Diagnostic(crate::framework::Tool::Gc);
        transfer.query = "gc".into();
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_log_handoff_prepare(&mut self, round: usize) {
        self.page = Page::Services;
        self.startup_warning = None;
        self.notification.clear();
        self.handoff = None;
        self.log_auto = false;
        let text = match round {
            0 => "noise\n[0.1s][gc] GC(0) Pause Young 10M->2M(20M) 1.0ms\n",
            1 => {
                "Traceback (most recent call last):\n  File \"demo.py\", line 1, in demo\nValueError: example\n"
            }
            _ => {
                "Exception in thread \"main\" java.lang.IllegalStateException: example\n\tat demo.Main.run(Main.java:1)\n"
            }
        };
        fs::write(
            self.manager
                .config_snapshot()
                .state_dir
                .join("logs/demo.log"),
            text,
        )
        .unwrap();
        self.request_logs("demo".into());
        if round == 0 {
            self.log_filter = "[gc]".into();
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_log_handoff_position(&self, index: usize) -> egui::Pos2 {
        let transfer = self.handoff.as_ref().unwrap();
        if index == 0 {
            transfer.preview_log_target.unwrap().center()
        } else {
            transfer.preview_rects[index].unwrap().center()
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_log_handoff_check(&mut self, round: usize, phase: usize) {
        match phase {
            0 => {
                assert!(!self.log_inflight && !self.log_error);
                self.log_error = true;
                self.send_service_logs("demo", false);
                assert!(self.handoff.is_none());
                self.log_error = false;
                self.log_inflight = true;
                self.send_service_logs("demo", false);
                assert!(self.handoff.is_none());
                self.log_inflight = false;
                let filter = self.log_filter.clone();
                let target_input = self.tool_state.input.clone();
                self.send_service_logs("demo", true);
                assert!(self.handoff.is_some() && self.log_view.is_none());
                self.cancel_handoff();
                assert_eq!(self.log_view.as_deref(), Some("demo"));
                assert_eq!(self.log_filter, filter);
                assert_eq!(self.tool_state.input, target_input);
            }
            1 => {
                let transfer = self.handoff.as_mut().unwrap();
                assert!(self.log_view.is_none());
                if round == 0 {
                    assert!(!transfer.text.contains("noise"));
                    assert!(transfer.text.contains("[gc]"));
                }
                let snapshot = transfer.text.clone();
                self.log_text = "later log update".into();
                self.log_lines.invalidate();
                assert_eq!(transfer.text, snapshot);
                transfer.query = match round {
                    0 => crate::framework::Tool::Gc.id().into(),
                    1 => ToolKind::DjangoTrace.id().into(),
                    _ => ToolKind::JavaTrace.id().into(),
                };
            }
            2 => {
                assert!(self.handoff.is_none());
                if round == 0 {
                    assert_eq!(self.page, Page::Java);
                    self.frameworks
                        .preview_received_log(crate::framework::Tool::Gc, "[gc]");
                } else {
                    assert_eq!(
                        self.tool_state.selected,
                        if round == 1 {
                            ToolKind::DjangoTrace
                        } else {
                            ToolKind::JavaTrace
                        }
                    );
                    assert!(self.tool_state.output.is_empty());
                    assert!(self.tool_state.input.contains(if round == 1 {
                        "Traceback"
                    } else {
                        "Exception in thread"
                    }));
                    assert!(
                        run_tool(
                            self.tool_state.selected,
                            0,
                            &self.tool_state.input,
                            "",
                            None,
                            10
                        )
                        .is_ok()
                    );
                }
            }
            _ => unreachable!(),
        }
    }
    pub(super) fn handoff_source(&self) -> Option<(String, &str)> {
        match self.page {
            Page::SmallTools | Page::EncodingTools => Some((
                self.tool_state.selected.label().into(),
                &self.tool_state.output,
            )),
            Page::Clipboard => self.clipboard.transfer_text(),
            Page::Files => Some(("文件校验报告".into(), self.file_state.report())),
            Page::Images => self.images.workflow_ocr_handoff_text().or_else(|| {
                self.images.image_report().map(|text| {
                    (
                        format!("{} 报告（冻结快照）", self.images.active_tool_id()),
                        text,
                    )
                })
            }),
            Page::Data => Some(("数据工作台导出".into(), &self.data_state.output)),
            Page::Notes | Page::Calendar => self.planner.transfer_text(),
            Page::Diff => Some(("文本差异报告".into(), &self.diff_state.diff_output)),
            Page::Network => Some(("网络诊断报告".into(), &self.network_state.output)),
            Page::Java | Page::Django => Some((
                format!("{}诊断报告", self.frameworks.selected.category()),
                self.frameworks.result_text(),
            )),
            Page::Agent => self
                .agent
                .handoff_answer()
                .map(|answer| ("Agent 最终回答".into(), answer)),
            Page::Http => self
                .http_state
                .tabs
                .get(self.http_state.selected)
                .map(|tab| {
                    if tab.handoff_body_only {
                        match tab.response.as_ref() {
                            Some(response) => (
                                format!(
                                    "HTTP 响应正文 · {}{}",
                                    response.content_type,
                                    if response.truncated {
                                        " · 预览已截断"
                                    } else {
                                        ""
                                    }
                                ),
                                if response.body_handoff_available {
                                    response.body.as_str()
                                } else {
                                    ""
                                },
                            ),
                            None => ("HTTP 响应正文".into(), ""),
                        }
                    } else {
                        (
                            "HTTP 完整响应（含状态与响应头）".into(),
                            tab.output.as_str(),
                        )
                    }
                }),
            _ => None,
        }
    }

    fn handoff_tool_id(&self) -> Option<String> {
        let id = match self.page {
            Page::SmallTools | Page::EncodingTools => self.tool_state.selected.id(),
            Page::Data => self.data_state.active_tool_id(),
            Page::Images => self.images.active_tool_id(),
            Page::Java | Page::Django => self.frameworks.selected.id(),
            Page::Agent => "agent-runner",
            Page::Plugins => "plugins",
            Page::Mcp => "mcp-inspector",
            Page::Notes => "memos",
            Page::Calendar => "calendar-planner",
            Page::Calculator => "advanced-calculator",
            Page::Clipboard => "super-clipboard",
            Page::Clock => "clock-workbench",
            Page::Files => "files",
            Page::Http => "http",
            Page::Diff => "diff",
            Page::Network => "network",
            Page::FileEncoding => "encoding-detect",
            Page::Markdown => "markdown",
            Page::ChecksumManifest => "checksum-manifest",
            Page::DiskInspector => "disk-inspector",
            Page::DuplicateFinder => "duplicate-files",
            Page::DirectoryCompare => "file-compare",
            Page::SqliteBrowser => "sqlite",
            Page::AsciiCodes => "ascii-codes",
            Page::Symbols => "symbol-library",
            Page::AsciiArt => "ascii-art",
            Page::KnowledgeSources => "rag-sources",
            Page::DocumentIngestion => "rag-ingestion",
            Page::KnowledgeIndex => "rag-index",
            Page::KnowledgeSearch => "rag-keyword-search",
            Page::VectorIndex => "rag-vectors",
            Page::HybridSearch => "rag-search",
            Page::KnowledgeAnswer => "rag-answer",
            Page::Embedding => "embedding-playground",
            Page::KnowledgeCapture => "browser-clipper",
            Page::KnowledgeEval => "rag-eval",
            Page::KnowledgeMcp => "mcp-server",
            Page::Recorder => "screen-recorder",
            _ => return None,
        };
        Some(id.to_owned())
    }
    pub(super) fn handoff_bar(&mut self, ctx: &egui::Context) {
        if self.page == Page::Calculator && self.calculator.date_output_active() {
            return;
        }
        if self.page == Page::Calculator && self.calculator.plot_active() {
            let result = self.calculator.plot_description();
            let mut send = false;
            egui::TopBottomPanel::top("result-handoff").show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let response = ui.add_enabled(
                        result.is_ok() && self.handoff.is_none(),
                        egui::Button::new("发送采样表到工具…"),
                    );
                    send = response.clicked();
                    #[cfg(feature = "ui-preview")]
                    {
                        self.calculator.preview_numeric_send = Some(response.rect);
                    }
                    match &result {
                        Ok(description) => {
                            ui.small(description);
                        }
                        Err(e) => {
                            ui.small(e);
                        }
                    }
                });
            });
            if send {
                match Transfer::new(
                    format!("函数采样表 · 计算器v{} · 近似值", calculator_version()),
                    self.calculator.plot_csv().unwrap(),
                ) {
                    Ok(mut transfer) => {
                        transfer.target = Target::Csv;
                        transfer.new_data_instance = true;
                        self.handoff = Some(transfer);
                    }
                    Err(e) => self.toast = Some((e.to_string(), Instant::now())),
                }
            }
            return;
        }
        if self.page == Page::Calculator {
            let result = self.calculator.numeric_description();
            let mut send = false;
            egui::TopBottomPanel::top("result-handoff").show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let response = ui.add_enabled(
                        result.is_ok() && self.handoff.is_none(),
                        egui::Button::new("发送数值到工具…"),
                    );
                    send = response.clicked();
                    #[cfg(feature = "ui-preview")]
                    {
                        self.calculator.preview_numeric_send = Some(response.rect);
                    }
                    match &result {
                        Ok(table) => {
                            ui.small(format!("{table} · 发送时捕获快照，不固定赋值"));
                        }
                        Err(error) => {
                            ui.small(error);
                        }
                    }
                });
            });
            if send {
                match Transfer::numeric(
                    format!("计算器结果快照 · v{}", calculator_version()),
                    self.calculator.numeric_result().unwrap(),
                ) {
                    Ok(t) => self.handoff = Some(t),
                    Err(e) => self.toast = Some((e.to_string(), Instant::now())),
                }
            }
            return;
        }
        let artifact_image = if self.page == Page::Artifacts {
            self.artifact_hub.handoff_image_snapshot()
        } else {
            None
        };
        let image_material = if self.page == Page::Images {
            self.images.workflow_handoff_image()
        } else {
            artifact_image
                .as_ref()
                .map(|(_, material)| material.clone())
        };
        let source = match self.page {
            Page::Mcp => self.mcp.handoff_result(),
            Page::Plugins => self.plugins.handoff_result(),
            Page::Artifacts => self.artifact_hub.handoff_snapshot(),
            _ => self
                .handoff_source()
                .map(|(source, text)| (source, text.to_owned())),
        };
        let use_image_material = source.is_none() && image_material.is_some();
        let source = source.or_else(|| {
            image_material.as_ref().map(|material| {
                (
                    artifact_image.as_ref().map_or_else(
                        || "图片流程图像结果".to_owned(),
                        |(source, _)| source.clone(),
                    ),
                    Material::Image(material.clone()).summary(),
                )
            })
        });
        let Some((source, text)) = source else {
            return;
        };
        let mut clicked = false;
        let body_source = matches!(self.page, Page::Notes | Page::Calendar);
        egui::TopBottomPanel::top("result-handoff").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                let response = ui.add_enabled(
                    (!text.is_empty() || image_material.is_some()) && self.handoff.is_none(),
                    egui::Button::new(if body_source {
                        "发送正文到工具…"
                    } else {
                        "发送结果到工具…"
                    }),
                );
                clicked = response.clicked();
                #[cfg(feature = "ui-preview")]
                if self.page == Page::Images {
                    ui.ctx().data_mut(|data| {
                        data.insert_temp(egui::Id::new("image-report-send"), response.rect)
                    });
                }
                ui.small(if text.is_empty() {
                    "生成结果后，可交给其他工具继续处理"
                } else if self.page == Page::Mcp {
                    "MCP 返回内容可能含敏感信息；请检查预览后再确认接收"
                } else if self.page == Page::Plugins {
                    "插件结果可能含敏感信息；请检查预览后再确认接收"
                } else {
                    "预览结果 → 选择目标 → 继续处理"
                });
            });
        });
        if clicked {
            let source = versioned_handoff_source(source, self.page);
            let source_reference = if self.page == Page::Artifacts {
                self.artifact_hub.handoff_source_reference()
            } else {
                None
            };
            let table = if self.page == Page::Artifacts {
                match self.artifact_hub.handoff_table_snapshot() {
                    Ok(table) => table,
                    Err(error) => {
                        self.toast = Some((error.to_string(), Instant::now()));
                        return;
                    }
                }
            } else {
                None
            };
            let transfer = if use_image_material {
                let Some(material) = image_material else {
                    return;
                };
                Transfer::material(source, Material::Image(material))
            } else {
                Transfer::artifact_snapshot(source, text.clone(), table)
            };
            let source_provenance = if self.page == Page::Artifacts {
                None
            } else {
                self.handoff_tool_id()
                    .and_then(|tool_id| crate::tool_provenance::current_for_tool(&tool_id).ok())
            };
            match transfer {
                Ok(mut transfer) => {
                    transfer.source_provenance = source_provenance;
                    transfer.source_reference = source_reference;
                    self.handoff = Some(transfer);
                }
                Err(error) => self.toast = Some((error.to_string(), Instant::now())),
            }
        }
    }
    pub(super) fn handoff_dialog(&mut self, ctx: &egui::Context) {
        if self.handoff.is_none()
            && let Some((source, table)) = self.data_state.take_numeric_request()
        {
            match Transfer::numeric(source, table) {
                Ok(mut transfer) => {
                    transfer.target = Target::Calculator;
                    self.handoff = Some(transfer);
                }
                Err(e) => self.toast = Some((e.to_string(), Instant::now())),
            }
        }
        let Some(transfer) = self.handoff.as_mut() else {
            return;
        };
        if transfer.access.is_none() {
            transfer.access = Some(discovery::Access::from_preferences(&self.preferences));
            transfer.matches_key = None;
        }
        transfer.discard_old_wheel(ctx, transfer.reset_body_scroll);
        let mut apply = false;
        let mut cancel = false;
        let mut focus_confirmation = false;
        let response = egui::Modal::new(egui::Id::new("handoff-modal"))
            .frame(egui::Frame::window(&ctx.style()).fill(self.colors.card))
            .show(ctx, |ui| {
            ui.set_width(480.0_f32.min(ctx.screen_rect().width() - 64.0));
            let footer_height = if transfer.target == Target::Calculator { 300.0 } else { 150.0 } + if transfer.error.is_empty() { 0.0 } else { 70.0 };
            let mut body = egui::ScrollArea::vertical().id_salt("handoff-body").max_height((ctx.screen_rect().height()-footer_height).max(100.0));
            if std::mem::take(&mut transfer.reset_body_scroll) { reset_scroll_area(ui,"handoff-body"); body = body.vertical_scroll_offset(0.0); }
            body.show(ui,|ui| {
            ui.heading("发送结果到工具");
            if let Some(table)=&transfer.numeric {
                ui.label(format!("数值快照：{}",table.description()));
                ui.horizontal_wrapped(|ui| {for (index,mode) in Representation::ALL.into_iter().enumerate() {let response=ui.selectable_value(&mut transfer.representation,mode,mode.label());
                    #[cfg(feature="ui-preview")] {transfer.numeric_mode_rects[index]=Some(response.rect);}
                    #[cfg(not(feature="ui-preview"))] {let _=(index,response);}
                }});
                ui.small(match transfer.representation {Representation::Typed=>"规范分子/分母字符串与完整f64类型。只有这种JSON表格可以无损送回矩阵。",Representation::Text=>"保留完整数值文本（近似值带≈），JSON/CSV/TSV不再保留数值类型；回传矩阵需保留类型。",Representation::Approximate=>"明确转换为近似f64：大整数、分数可能损失精度；JSON/CSV/TSV无法恢复原精确值。"});
            }
            if transfer.target == Target::Calculator
                && transfer.numeric.is_none()
                && let (Some(Material::Table(dataset)), Some(table)) =
                    (&transfer.material, &transfer.table_numeric)
            {
                ui.small(format!(
                    "将完整表格 {}×{} 映射到{}；矩阵列按表格列顺序排列，表头名称不会作为数据传入。{}",
                    table.rows,
                    table.cols,
                    transfer.matrix_slot.label(),
                    table.description()
                ));
                let columns = dataset.headers.join(" → ");
                ui.small(format!(
                    "列顺序：{}",
                    columns.chars().take(256).collect::<String>()
                ));
            }
            transfer.refresh_numeric();
            if let Some(Material::Table(dataset)) = &transfer.material
                && transfer.table_numeric.is_none()
            {
                ui.small(format!(
                    "计算器目标仅适用于完整1–8行×1–8列数值矩形；当前表格为{}行×{}列或含非数值单元格。可到数据工作台选取需要的行列范围。",
                    dataset.rows.len(),
                    dataset.headers.len()
                ));
            }
            if let Some(Material::File(file)) = &transfer.material {
                ui.label(format!("来源：{} · 文件 {} · {} 字节", transfer.source, file.display_name(), file.bytes()));
                ui.small(if transfer.target == Target::TextFlow {
                    "创建独立工具流程并添加文件 SHA-256 首步。接收只固定文件引用；点击运行后才分块读取和计算，不显示完整路径，也不修改原文件。"
                } else {
                    "只传递文件引用；不读取内容。接收时重新核验身份，之后仍需在编码检查中主动点击读取。"
                });
                if transfer.target == Target::FileEncoding
                    && file.bytes() > crate::file_encoding::MAX_INPUT as u64
                {
                    ui.colored_label(self.colors.red, "文件超过编码检查 8 MiB 上限，无法接收");
                }
            } else {
                ui.label(format!("来源：{} · {} 字节", transfer.source, transfer.text.len()));
            }
            if let Some(material) = &transfer.material {
                ui.small(material.summary());
            }
            if transfer.material.is_none() {
                ui.small("预览最多 1200 字符；发送完整快照。此内容仅保留在本次内存中。");
            }
            egui::ScrollArea::vertical().id_salt("handoff-preview").max_height(100.0).show(ui, |ui| {
                ui.monospace(&transfer.preview);
            });
            ui.separator();
            ui.horizontal_wrapped(|ui| {
                for scope in discovery::Scope::ALL {
                    let _response = ui.selectable_value(&mut transfer.scope, scope, scope.label());
                    #[cfg(feature="ui-preview")]
                    ui.ctx().data_mut(|data|data.insert_temp(egui::Id::new(("handoff-scope-proof",scope.label())),_response.rect));
                }
            });
            let navigation_action = transfer.navigation.input(ctx,
                ui.memory(|m| m.has_focus(egui::Id::new("handoff-query-input")) && m.top_modal_layer() == Some(ui.layer_id()))
                && ctx.input(|i| i.raw.focused)
                && !egui::Popup::is_any_open(ctx));
            let focus_search = std::mem::take(&mut transfer.focus_query)
                || (ui.memory(|m| m.top_modal_layer() == Some(ui.layer_id()))
                    && !egui::Popup::is_any_open(ctx)
                    && ui.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::K)));
            let _query_response = ui.add(egui::TextEdit::singleline(&mut transfer.query)
                .id(egui::Id::new("handoff-query-input"))
                .hint_text("搜索目标 · Ctrl+K · 例如 JSON、对比、备忘录"));
            if focus_search { _query_response.request_focus(); }
            #[cfg(feature="ui-preview")]
            ui.ctx().data_mut(|data|data.insert_temp(egui::Id::new("handoff-query-proof"),_query_response.rect));
            let _category_response = egui::ComboBox::from_id_salt("handoff-category")
                .selected_text(if transfer.category.is_empty() { "全部分类" } else { &transfer.category })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut transfer.category, String::new(), "全部分类");
                    for category in discovery::categories() {
                        ui.selectable_value(&mut transfer.category, category.clone(), category);
                    }
                });
            #[cfg(feature="ui-preview")]
            ui.ctx().data_mut(|data|data.insert_temp(egui::Id::new("handoff-category-proof"),_category_response.response.rect));
            let targets = transfer.matching_targets();
            let recommended: Vec<_> = if transfer.scope == discovery::Scope::All && transfer.query.trim().is_empty() {
                transfer.recommendations.iter().filter(|item| targets.contains(&item.target)).map(|item|item.target).collect()
            } else { Vec::new() };
            let ordered = keyboard::ordered(&targets,&recommended);
            let reveal = match navigation_action {
                Some(keyboard::Action::Next) => transfer.navigation.step(&ordered,false),
                Some(keyboard::Action::Previous) => transfer.navigation.step(&ordered,true),
                Some(keyboard::Action::FocusConfirmation) => {
                    focus_confirmation = transfer.navigation.cursor.is_some_and(|target|targets.contains(&target));
                    None
                },
                None => None,
            };
            if let Some(target) = reveal { transfer.target=target; transfer.error.clear(); }
            ui.small(format!("{} 个目标 · ↑↓ 选择 · 选中后 Tab 到确认", targets.len()))
                .on_hover_text("支持名称、别名、分类和用途搜索；确认按钮才会接收内容。");
            if transfer.scope != discovery::Scope::All {
                ui.small("沿用工具库的收藏与使用记录，仅显示兼容的接力入口。");
            }
            let mut target_scroll = egui::ScrollArea::vertical().id_salt("handoff-targets").max_height(160.0);
            if std::mem::take(&mut transfer.reset_target_scroll) { transfer.discard_old_wheel(ctx, true); reset_scroll_area(ui,"handoff-targets"); target_scroll = target_scroll.vertical_scroll_offset(0.0); }
            let _target_scroll = target_scroll.show(ui, |ui| {
                if !recommended.is_empty() {
                    ui.small("推荐目标 · 仅提示，不自动执行");
                    for target in ordered.iter().copied().filter(|target|recommended.contains(target)) {
                        let _response = target_button(ui,transfer,target,reveal);
                        #[cfg(feature = "ui-preview")]
                        if matches!(target, Target::JsonData | Target::Csv | Target::Tsv) { transfer.preview_recommendation_rect = Some(_response.rect); }
                        let reason=transfer.recommendations.iter().find(|item|item.target==target).unwrap().reason;
                        ui.label(egui::RichText::new(reason).small().weak());
                    }
                    ui.separator();
                    ui.small("其他目标");
                }
                for target in targets.iter().copied().filter(|target| !recommended.contains(target)) {
                    let _response = target_button(ui,transfer,target,reveal);
                    #[cfg(feature = "ui-preview")]
                    if target == Target::Event { transfer.preview_rects[0] = Some(_response.rect); }
                    #[cfg(feature = "ui-preview")]
                    if matches!(target, Target::Diagnostic(_) | Target::Tool(ToolKind::JavaTrace | ToolKind::DjangoTrace)) { transfer.preview_log_target = Some(_response.rect); }
                }
                if targets.is_empty() {
                    ui.label("当前范围没有匹配的接收目标。已选目标保留在下方，尚未执行。");
                    let response = ui.button("清除筛选，查看全部");
                    #[cfg(feature="ui-preview")]
                    ui.ctx().data_mut(|data|data.insert_temp(egui::Id::new("handoff-clear-filter-proof"),response.rect));
                    if response.clicked() {
                        transfer.scope = discovery::Scope::All;
                        transfer.category.clear();
                        transfer.query.clear();
                    }
                }
            });
            #[cfg(feature="ui-preview")]
            ui.ctx().data_mut(|data| data.insert_temp(egui::Id::new("handoff-target-scroll-proof"), (_target_scroll.inner_rect, _target_scroll.state.offset.y)));
            ui.separator();
            transfer.refresh_numeric();
            let data_target = matches!(transfer.target, Target::Csv | Target::Tsv | Target::JsonData);
            if transfer.target == Target::Event {
                if let Some(date)=transfer.event_date { ui.label(format!("新日程：{date} 全天 · 不重复 · 提醒关闭")); }
                else { ui.label(format!("新日程：{} 09:00 · 本机时区 · 不重复 · 提醒关闭", self.planner.incoming_event_date())); }
            }
            if data_target && transfer.material.is_none() { ui.checkbox(&mut transfer.new_data_instance,"在新数据实例中打开，保留已有工作"); }
            ui.small(if transfer.target == Target::Agent {"加入独立接力资料区，最多32 KiB；保留任务目标。确认后只进入Agent草稿，只有手动生成计划时才发送给本机模型。"} else if matches!(transfer.target, Target::Diagnostic(_)) {"替换该诊断主输入并清除旧结果，保留对照输入与其他参数；同目标执行中拒绝接收。接收不自动分析或执行命令。"} else if matches!(transfer.material.as_ref(), Some(Material::Image(_))) && transfer.target == Target::ArtifactHub {"接收为本机资料库临时图片草稿。只有你继续预览并明确确认后才会编码并保存为 PNG；图片内容不会进入文本搜索。"} else if matches!(transfer.material.as_ref(), Some(Material::Image(_))) {"有界内存图片及来源链进入独立文本流程实例；图片仅保留在当前会话，不会写入工作实例快照。接收不会运行流程或保存图片。"} else if transfer.target == Target::Event {"完整结果作为日程正文，最多 128 KiB；已有备忘/日程编辑需先保存或放弃。打开后调整日期时间并主动开启提醒，再保存到本机。"} else if transfer.target == Target::Memo {"创建备忘草稿，最多 128 KiB；已有编辑需先保存或放弃。点击备忘录中的保存后才会写入本机。"} else if transfer.target == Target::Calculator && matches!(transfer.material.as_ref(), Some(Material::Table(_))) {"显式转换完整的1–8行、1–8列数值表到所选矩阵；精确分数和近似值类型按原单元格保留，不传递表头或执行计算。更大表格可在数据工作台选择行列范围。"} else if matches!(transfer.material.as_ref(), Some(Material::Table(_))) {"按结构化表格材料新建独立数据实例；保留单元格类型、列顺序和已有工作，不运行转换或自动保存。"} else if matches!(transfer.material.as_ref(), Some(Material::File(_))) {"接收时重新核验文件身份；不读取、转换或保存，直到你在编码检查中主动点击读取。"} else if data_target && transfer.new_data_instance {"创建新实例并解析预览；当前工作和原结果保留。"} else {"将替换目标输入并清除旧结果，保留其他参数。数据工作台会解析预览，其他工具需手动运行。"});
            if transfer.target == Target::ArtifactHub {
                ui.small("只把结果放入未保存草稿；进入资料库后查看内容，再单独确认写入本机。旧版本会保留，冲突时拒绝覆盖。");
            } else if transfer.target == Target::KnowledgeCapture {
                ui.small("最多 256 KiB；只填入网页与对话收集的内存草稿。接收后需准备正文预览，并再次确认保存到所选知识源；不会自动扫描或更新索引。");
            }
            });
            ui.separator();
            ui.label(format!("目标：{}", transfer.target.label()));
            if let Some(date)=transfer.event_date {
                ui.label(format!("日期快照：{date} · 日程目标使用此日期，不使用当前选中日期"));
                if transfer.target==Target::Event { ui.small("创建单次全天草稿，提醒关闭，不自动保存。日程支持1901–2099年，范围外可选择备忘录或文本目标。"); }
            }

            if transfer.target==Target::Calculator {
                let previous_slot=transfer.matrix_slot;
                ui.horizontal_wrapped(|ui| {ui.label("接收位置");let a=ui.selectable_value(&mut transfer.matrix_slot,MatrixSlot::A,"矩阵A");let b=ui.selectable_value(&mut transfer.matrix_slot,MatrixSlot::B,"矩阵B");
                    #[cfg(feature="ui-preview")] {transfer.matrix_rects[0]=Some(a.rect);transfer.matrix_rects[1]=Some(b.rect);}
                    #[cfg(not(feature="ui-preview"))] {let _=(a,b);}});
                if previous_slot!=transfer.matrix_slot {transfer.allow_matrix_replace=false;}
                if self.calculator.dirty() {let _response=ui.checkbox(&mut transfer.allow_matrix_replace,format!("允许替换未保存的{}（其他工作保留）",transfer.matrix_slot.label()));
                    #[cfg(feature="ui-preview")] {transfer.matrix_rects[2]=Some(_response.rect);}}
                match NumericTable::read_json(&transfer.text) {Ok(table)=>{ui.label(format!("将替换{}：{}",transfer.matrix_slot.label(),table.description()));},Err(e)=>{ui.colored_label(self.colors.red,e);}}
                ui.small("仅接受c1..cN规范数值类型JSON；未保存内容须先保存或明确允许替换所选矩阵；待读取或后台I/O须先处理。保留另一矩阵、算式和变量，不自动计算；接收后标记未保存。");
            }
            if !transfer.error.is_empty() {
                egui::ScrollArea::vertical().id_salt("handoff-error").max_height(56.0).show(ui, |ui| { ui.colored_label(self.colors.red, &transfer.error); });
            }
            let data_target=matches!(transfer.target,Target::Csv|Target::Tsv|Target::JsonData);
            ui.horizontal(|ui| {
                cancel = ui.button("取消").clicked();
                let response = ui.add_enabled(transfer.compatible(),egui::Button::new(if transfer.target == Target::Agent {"加入接力资料并打开"} else if transfer.target == Target::FileEncoding {"接收文件引用并打开"} else if transfer.target == Target::ArtifactHub {"打开未保存资料草稿"} else if transfer.target == Target::KnowledgeCapture {"打开临时收集草稿"} else if transfer.target == Target::Event {"创建日程草稿"} else if transfer.target == Target::Memo {"创建备忘草稿"} else if transfer.target == Target::TextFlow {"新建流程实例并打开"} else if data_target && transfer.new_data_instance {"新建实例并打开"}else{"替换输入并打开"}));
                apply = response.clicked();
                if focus_confirmation { response.request_focus(); }
                #[cfg(feature = "ui-preview")]
                { transfer.preview_rects[1] = Some(response.rect); ui.ctx().data_mut(|data| {
                    data.insert_temp(egui::Id::new("handoff-apply"),response.rect);
                    data.insert_temp(egui::Id::new("handoff-apply-focus-proof"),response.id);
                }); }
            });
        });
        // Let the topmost popup/modal handle Escape first: closing a category
        // popup must not discard the surrounding handoff in the same frame.
        if response.should_close() {
            cancel = true;
        }
        if cancel {
            self.cancel_handoff();
        } else if apply {
            self.apply_handoff();
        }
    }
    pub(super) fn finish_date_transfer(&mut self) {
        match self.calculator.take_date_transfer() {
            Ok(Some((date, text))) if self.handoff.is_none() => match Transfer::dated(
                format!("计算器日期结果 · v{}", calculator_version()),
                date,
                &text,
            ) {
                Ok(transfer) => self.handoff = Some(transfer),
                Err(error) => self.toast = Some((error.to_string(), Instant::now())),
            },
            Err(error) => self.toast = Some((error, Instant::now())),
            _ => {}
        }
    }
    fn apply_handoff(&mut self) {
        if let Some(transfer) = self.handoff.as_ref() {
            if !transfer.compatible() {
                self.handoff.as_mut().unwrap().error = if matches!(
                    transfer.material.as_ref(),
                    Some(Material::File(file)) if transfer.target == Target::FileEncoding && file.bytes() > crate::file_encoding::MAX_INPUT as u64
                ) {
                    "文件超过编码检查8 MiB上限，不能接收".into()
                } else {
                    "当前材料类型与所选目标不兼容".into()
                };
                return;
            }
            let result = if let Some(Material::File(file)) = transfer.material.as_ref() {
                if transfer.target == Target::TextFlow {
                    self.data_state
                        .import_text_flow_file(file.clone(), "接力文件流程")
                        .map(|_| (Page::Data, None))
                } else {
                    transfer.apply_file(&mut self.file_encoding)
                }
            } else if transfer.target == Target::ArtifactHub
                && let Some(Material::Image(image)) = transfer.material.as_ref()
            {
                self.artifact_hub
                    .receive_image_with_reference(
                        image.clone(),
                        transfer.source_provenance.as_ref(),
                        transfer.source_reference.as_deref(),
                    )
                    .map(|_| (Page::Artifacts, None))
            } else if matches!(transfer.material.as_ref(), Some(Material::Image(_))) {
                transfer.apply_image(&mut self.data_state)
            } else if transfer.target == Target::Calculator {
                self.calculator
                    .receive_numeric_with_policy(
                        &transfer.text,
                        transfer.matrix_slot,
                        transfer.allow_matrix_replace,
                    )
                    .map(|_| (Page::Calculator, None))
                    .map_err(anyhow::Error::msg)
            } else if transfer.material.is_some() {
                if transfer.target == Target::ArtifactHub {
                    self.artifact_hub
                        .receive_text_with_reference(
                            &transfer.text,
                            transfer.artifact_kind(),
                            transfer.source_provenance.as_ref(),
                            transfer.source_reference.as_deref(),
                        )
                        .map(|_| (Page::Artifacts, None))
                } else {
                    transfer.apply_material(&mut self.data_state)
                }
            } else if transfer.target == Target::KnowledgeCapture {
                let source = transfer
                    .source_reference
                    .clone()
                    .or_else(|| {
                        transfer.source_provenance.as_ref().map(|provenance| {
                            format!("{}@{}", provenance.tool_id, provenance.tool_version)
                        })
                    })
                    .unwrap_or_else(|| "Zi DevTools 工具接力".into());
                self.knowledge_capture
                    .receive_handoff(&source, &transfer.text)
                    .map(|_| (Page::KnowledgeCapture, None))
            } else if let Target::Diagnostic(tool) = transfer.target {
                self.frameworks
                    .receive_handoff(tool, transfer.text.clone())
                    .map(|_| {
                        (
                            if tool.category() == "Java 与 JVM" {
                                Page::Java
                            } else {
                                Page::Django
                            },
                            None,
                        )
                    })
            } else if transfer.target == Target::Event {
                (if let Some(date) = transfer.event_date {
                    self.planner
                        .receive_event_date(&transfer.source, &transfer.text, date)
                } else {
                    self.planner
                        .receive_event_text(&transfer.source, &transfer.text)
                })
                .map(|_| (Page::Calendar, None))
            } else if transfer.target == Target::Memo {
                self.planner
                    .receive_text(&transfer.source, &transfer.text)
                    .map(|_| (Page::Notes, None))
            } else if transfer.target == Target::Agent {
                self.agent
                    .receive_handoff(&transfer.source, &transfer.text)
                    .map(|_| (Page::Agent, None))
            } else if transfer.target == Target::TextFlow {
                self.data_state
                    .import_text_flow(transfer.text.clone(), "接力文本流程")
                    .map(|_| (Page::Data, None))
            } else if transfer.new_data_instance
                && matches!(
                    transfer.target,
                    Target::Csv | Target::Tsv | Target::JsonData
                )
            {
                self.data_state
                    .import_new(
                        transfer.text.clone(),
                        if transfer.target == Target::JsonData {
                            crate::workbench::DataFormat::Json
                        } else {
                            crate::workbench::DataFormat::Csv
                        },
                        transfer.target == Target::Tsv,
                        "接力数据",
                    )
                    .map(|_| (Page::Data, None))
            } else {
                transfer.apply(
                    &mut self.tool_state,
                    &mut self.data_state,
                    &mut self.diff_state,
                    &mut self.artifact_hub,
                )
            };
            match result {
                Ok((page, kind)) => {
                    let message = if matches!(transfer.material.as_ref(), Some(Material::Image(_)))
                    {
                        "图片已接入新的文本流程；点击运行后才会生成信息报告"
                    } else if transfer.target == Target::KnowledgeCapture {
                        "结果已放入知识收集草稿；尚未保存、扫描或更新索引"
                    } else if matches!(transfer.material.as_ref(), Some(Material::File(_))) {
                        if transfer.target == Target::TextFlow {
                            "文件引用已加入新流程；尚未读取"
                        } else {
                            "文件引用已接收；尚未读取"
                        }
                    } else {
                        "结果已填入目标工具"
                    };
                    // Multiple receivers share one workbench. Attribute the
                    // successful handoff to its explicit destination, once.
                    let receiving_tool_id = transfer.target.entry().id.as_str();
                    self.handoff = None;
                    self.navigate_with_visit(page, kind, Some(receiving_tool_id));
                    self.qr_texture = None;
                    self.toast = Some((message.into(), Instant::now()));
                }
                Err(error) => self.handoff.as_mut().unwrap().error = error.to_string(),
            }
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_common_image_handoff_prepare(&mut self, ctx: &egui::Context, light: bool) {
        self.set_theme(ctx, if light { Theme::Light } else { Theme::Dark });
        self.startup_warning = None;
        self.page = Page::Images;
        self.images.preview_common_handoff_fixture();
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_common_image_handoff_apply_position(&self) -> egui::Pos2 {
        self.handoff
            .as_ref()
            .expect("image result handoff review")
            .preview_rects[1]
            .expect("rendered handoff apply button")
            .center()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_common_image_handoff_check(&self, received: bool) {
        if received {
            assert!(self.handoff.is_none());
            assert_eq!(self.page, Page::Data);
            assert_eq!(self.data_state.instances.len(), 2);
            assert_eq!(self.data_state.active_id(), self.data_state.instances[1].id);
            assert_eq!(self.data_state.active_tool_id(), "text-flow");
            assert!(!self.data_state.busy());
            assert!(self.images.workflow_handoff_image().is_some());
        } else {
            assert_eq!(self.page, Page::Images);
            let transfer = self.handoff.as_ref().expect("image result handoff review");
            assert_eq!(transfer.target, Target::TextFlow);
            assert!(transfer.compatible());
            assert_eq!(transfer.recommendations[0].target, Target::TextFlow);
            assert!(transfer.preview.contains("64 × 48"));
            assert_eq!(
                transfer.source_provenance.as_ref(),
                Some(&crate::tool_provenance::current_for_tool("image-workflow").unwrap())
            );
            assert_eq!(self.data_state.instances.len(), 1);
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_handoff_discovery_position(&self, index: usize) -> egui::Pos2 {
        let transfer = self.handoff.as_ref().unwrap();
        if index == 0 {
            transfer.preview_recommendation_rect.unwrap().center()
        } else {
            transfer.preview_rects[1].unwrap().center()
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_handoff_query_check(&self, expected: &str) {
        assert_eq!(self.handoff.as_ref().unwrap().query, expected);
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_handoff_access_prepare(&mut self, ctx: &egui::Context, light: bool) {
        self.preview_delimited_handoff_prepare(ctx, light, false);
        self.preferences.favorites = vec!["data".into()];
        for _ in 0..10 {
            self.preferences.visit("data");
        }
        for _ in 0..7 {
            self.preferences.visit("memos");
        }
        // The app refreshes discovery preferences from disk. This fixture uses
        // its disposable service-config directory, never the user's preferences.
        self.preferences.save(&self.preferences_path).unwrap();
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_handoff_access_check(&mut self, phase: u8) {
        if phase == 7 {
            self.preview_delimited_handoff_check(3, false);
            return;
        }
        self.preview_delimited_handoff_check(if phase < 2 { 0 } else { 1 }, false);
        assert_eq!(self.preferences.recent, ["memos", "data"]);
        assert_eq!(self.preferences.usage["data"], 10);
        let transfer = self.handoff.as_mut().unwrap();
        let targets = transfer.matching_targets();
        match phase {
            0 => assert_eq!(transfer.scope, discovery::Scope::All),
            1 | 2 => {
                assert_eq!(transfer.scope, discovery::Scope::Favorites);
                assert_eq!(targets.len(), 3);
                assert_eq!(targets[0], Target::Csv);
            }
            3 => {
                assert_eq!(transfer.scope, discovery::Scope::Recent);
                assert_eq!(targets[0], Target::Memo);
            }
            4 => {
                assert_eq!(transfer.scope, discovery::Scope::Frequent);
                assert_eq!(targets[0], Target::Csv);
            }
            5 => {
                assert_eq!(transfer.query, "unmatched987654");
                assert!(targets.is_empty());
            }
            6 => {
                assert_eq!(transfer.scope, discovery::Scope::All);
                assert!(transfer.query.is_empty() && transfer.category.is_empty());
                assert!(targets.len() > 10);
            }
            _ => unreachable!(),
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_handoff_visit_check(&mut self) {
        for (target, id, text) in [
            (Target::TextFlow, "text-flow", "unchanged relay text"),
            (
                Target::Diagnostic(crate::framework::Tool::Gc),
                "java-gc-log",
                "[gc] synthetic GC input",
            ),
            (
                Target::Diagnostic(crate::framework::Tool::Sql),
                "django-sql",
                "SELECT synthetic_fixture",
            ),
        ] {
            let mut expected_usage = self.preferences.usage.clone();
            *expected_usage.entry(id.into()).or_default() += 1;
            let mut transfer = Transfer::new("attribution fixture".into(), text).unwrap();
            transfer.target = target;
            self.handoff = Some(transfer);
            self.apply_handoff();
            assert!(self.handoff.is_none(), "successful handoff: {id}");
            assert_eq!(
                self.preferences.recent.first().map(String::as_str),
                Some(id),
                "handoff must visit the exact receiving tool"
            );
            assert_eq!(
                self.preferences.usage, expected_usage,
                "exactly one visit; no generic workbench count"
            );
            let saved = Preferences::load(&self.preferences_path);
            assert_eq!(saved.recent, self.preferences.recent);
            assert_eq!(saved.usage, expected_usage);
            match target {
                Target::TextFlow => {
                    assert!(self.page == Page::Data);
                    assert_eq!(self.data_state.active_tool_id(), "text-flow");
                }
                Target::Diagnostic(tool) => {
                    assert_eq!(self.frameworks.selected, tool);
                    self.frameworks.preview_imported_draft(tool, text);
                }
                _ => unreachable!(),
            }
        }
        let usage = self.preferences.usage.clone();
        let recent = self.preferences.recent.clone();
        let mut invalid = Transfer::new("failure fixture".into(), "not numeric JSON").unwrap();
        invalid.target = Target::Calculator;
        self.handoff = Some(invalid);
        self.apply_handoff();
        assert!(!self.handoff.as_ref().unwrap().error.is_empty());
        assert_eq!(self.preferences.usage, usage);
        assert_eq!(self.preferences.recent, recent);
        self.cancel_handoff();
        assert!(self.handoff.is_none());
        assert_eq!(self.preferences.usage, usage);
        assert_eq!(self.preferences.recent, recent);
        println!(
            "PASS exact handoff visits: TextFlow, Java GC, Django SQL; one persisted visit each; failure/cancel add none"
        );
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_delimited_handoff_prepare(
        &mut self,
        ctx: &egui::Context,
        light: bool,
        tsv: bool,
    ) {
        self.set_theme(ctx, if light { Theme::Light } else { Theme::Dark });
        self.startup_warning = None;
        self.page = Page::Data;
        self.data_state.input = "preserve existing data draft".into();
        self.data_state.output = "preserve existing export".into();
        self.tool_state.output = if tsv {
            "name\tcount\nZi\t2\n工具\t3"
        } else {
            "name,count\nZi,2\n工具,3"
        }
        .into();
        self.handoff = Some(Transfer::new("合成表格结果".into(), &self.tool_state.output).unwrap());
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_delimited_handoff_check(&self, phase: u8, tsv: bool) {
        let target = if tsv { Target::Tsv } else { Target::Csv };
        if phase < 2 {
            assert!(self.handoff.is_some(), "delimited handoff phase {phase}");
            let transfer = self.handoff.as_ref().unwrap();
            assert_eq!(transfer.recommendations[0].target, target);
            assert_eq!(
                transfer.target,
                if phase == 0 {
                    Target::Tool(ToolKind::Json)
                } else {
                    target
                }
            );
            assert_eq!(self.data_state.instances.len(), 1);
            assert_eq!(self.data_state.input, "preserve existing data draft");
            assert_eq!(self.data_state.output, "preserve existing export");
            assert!(!self.data_state.busy());
        } else if phase == 2 {
            assert!(self.handoff.is_none());
            assert_eq!(self.data_state.instances.len(), 1);
            assert_eq!(self.data_state.input, "preserve existing data draft");
            assert_eq!(self.data_state.output, "preserve existing export");
        } else {
            assert!(self.handoff.is_none());
            assert_eq!(self.data_state.instances.len(), 2);
            assert_eq!(
                self.data_state.instances[0].state.input,
                "preserve existing data draft"
            );
            assert_eq!(
                self.data_state.instances[0].state.output,
                "preserve existing export"
            );
            assert_eq!(self.data_state.input, self.tool_state.output);
            assert_eq!(self.data_state.parse_job.phase, crate::tasks::Phase::Done);
            let table: serde_json::Value =
                serde_json::from_str(&self.data_state.preview_numeric_export()).unwrap();
            assert_eq!(
                table,
                serde_json::json!([{"name":"Zi","count":"2"},{"name":"工具","count":"3"}])
            );
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_clipboard_json_records_handoff_smoke(&mut self, phase: u8) {
        if phase == 0 {
            self.clipboard.preview_json_records_fixture();
            self.page = Page::Clipboard;
            self.data_state.input = "existing data work".into();
            self.data_state.output = "original export".into();
            let (title, text) = self
                .handoff_source()
                .expect("clipboard fixture is transferable");
            self.handoff = Some(Transfer::new(title, text).unwrap());
        } else {
            assert!(self.handoff.is_none() && self.page == Page::Data);
            assert_eq!(self.data_state.instances.len(), 2);
            let old = &self.data_state.instances[0];
            assert_eq!(old.state.input, "existing data work");
            assert_eq!(old.state.output, "original export");
            assert_ne!(self.data_state.active_id(), old.id);
            let (_, source) = self
                .clipboard
                .transfer_text()
                .expect("clipboard source remains available");
            assert_eq!(self.data_state.input, source);
            assert!(self.data_state.format == crate::workbench::DataFormat::Json);
            assert_eq!(self.data_state.parse_job.phase, crate::tasks::Phase::Done);
            let records: serde_json::Value = serde_json::from_str(source).unwrap();
            let records = records.as_array().expect("record JSON is an array");
            assert_eq!(records.len(), 3);
            assert_eq!(records[0]["order"], 1);
            assert_eq!(records[0]["source"], "示例编辑器");
            assert_eq!(
                records[0]["text"],
                "教程第一步：准备示例数据\n仅使用合成内容"
            );
            assert_eq!(records[0]["labels"][0], "教程");
            assert_eq!(records[1]["order"], 2);
            assert_eq!(records[1]["source"], "示例终端");
            assert_eq!(records[2]["order"], 3);
            assert_eq!(records[2]["source"], "示例浏览器");
            assert!(records.iter().all(|record| record["time"].is_string()));
            println!(
                "PASS clipboard JSON-record handoff: actual recommendation click and explicit apply; parsed all 3 ordered records with source/time/labels; original data draft/export and clipboard source preserved"
            );
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_event_handoff_position(&self, index: usize) -> egui::Pos2 {
        self.handoff.as_ref().unwrap().preview_rects[index]
            .unwrap()
            .center()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_event_handoff_scene(&mut self) {
        self.preview_event_handoff_smoke(0);
        self.handoff.as_mut().unwrap().target = Target::Event;
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_event_handoff_smoke(&mut self, phase: u8) {
        if phase == 0 {
            self.preview_memo_handoff(false);
            let transfer = self.handoff.as_mut().unwrap();
            transfer.target = Target::Tool(ToolKind::Json);
            transfer.query = "日程".into();
        } else {
            assert!(self.handoff.is_none() && self.page == Page::Calendar);
            let source = self.tool_state.output.clone();
            self.planner.preview_incoming_event_assert(&source);
            let mut transfer = Transfer::new("another".into(), "must not overwrite").unwrap();
            transfer.target = Target::Event;
            self.handoff = Some(transfer);
            self.apply_handoff();
            assert!(!self.handoff.as_ref().unwrap().error.is_empty());
            self.planner.preview_incoming_event_assert(&source);
            self.handoff = None;
            println!(
                "PASS event handoff: actual target selection and create click; full source preserved; unsaved event, selected date at 09:00, reminder off; second handoff rejected; no database write"
            );
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_memo_handoff(&mut self, receive: bool) {
        self.planner.preview(false, false);
        let mut transfer = Transfer::new(
            "JSON 格式化结果".into(),
            "{\n  \"project\": \"Zi DevTools\",\n  \"status\": \"ready\"\n}",
        )
        .unwrap();
        transfer.target = Target::Memo;
        transfer.query = "备忘录".into();
        self.handoff = Some(transfer);
        self.page = Page::EncodingTools;
        self.tool_state.select(ToolKind::Json);
        self.tool_state.output = self.handoff.as_ref().unwrap().text.clone();
        if receive {
            self.apply_handoff();
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_memo_roundtrip(&mut self) {
        self.preview_memo_handoff(true);
        assert_eq!(self.page, Page::Notes);
        assert!(self.planner.has_unsaved());
        let source = self.tool_state.output.clone();
        let (title, text) = self.handoff_source().unwrap();
        assert_eq!(text, source);
        let mut transfer = Transfer::new(title, text).unwrap();
        transfer.target = Target::Tool(ToolKind::Base64);
        self.handoff = Some(transfer);
        self.apply_handoff();
        assert_eq!(self.tool_state.selected, ToolKind::Base64);
        assert_eq!(self.tool_state.input, source);
        assert!(self.tool_state.output.is_empty());
        self.tool_state.select(ToolKind::Json);
        assert_eq!(self.tool_state.output, source);
        let mut transfer = Transfer::new("新结果".into(), "replacement").unwrap();
        transfer.target = Target::Memo;
        self.handoff = Some(transfer);
        self.apply_handoff();
        assert!(self.handoff.as_ref().is_some_and(|t| !t.error.is_empty()));
        assert_eq!(self.planner.transfer_text().unwrap().1, source);
        self.handoff = None;
        self.planner.preview(false, false);
        println!(
            "PASS memo handoff: actual app routes, source preserved, unsaved draft guarded, outbound input without execution"
        );
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_workflow_table_handoff_check(&self, phase: u8) {
        assert_eq!(self.page, Page::Data);
        assert!(!self.data_state.instances.is_empty());
        self.data_state.instances[0]
            .state
            .preview_workflow_step_send_check();
        match phase {
            0 => {
                assert_eq!(self.data_state.instances.len(), 1);
                let transfer = self.handoff.as_ref().expect("typed table review open");
                assert_eq!(transfer.source, "pipeline v0.20.0 · 第1步结果");
                assert_eq!(transfer.target, Target::JsonData);
                assert!(transfer.preview_typed_table_ready());
                assert_eq!(
                    transfer.source_provenance.as_ref(),
                    Some(&crate::tool_provenance::current_for_tool("pipeline").unwrap())
                );
            }
            1 => {
                assert!(self.handoff.is_none());
                assert_eq!(self.data_state.instances.len(), 2);
                assert_eq!(self.data_state.active_id(), self.data_state.instances[1].id);
                assert_eq!(
                    self.data_state.instances[1].name,
                    "pipeline v0.20.0 · 第1步结果"
                );
                self.data_state.instances[1]
                    .state
                    .preview_workflow_step_send_received_check();
                self.data_state.instances[0]
                    .state
                    .preview_workflow_step_send_check();
            }
            _ => panic!("unknown workflow table handoff phase"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn artifact_state() -> crate::artifact_hub::State {
        crate::artifact_hub::State::new(std::env::temp_dir().join(format!(
            "zi-handoff-artifact-{}.sqlite3",
            uuid::Uuid::new_v4()
        )))
    }
    #[test]
    fn direct_agent_plugin_and_mcp_handoffs_display_catalog_versions() {
        assert_eq!(
            versioned_handoff_source("Agent 最终回答".into(), Page::Agent),
            "Agent 最终回答 · agent-runner v1.10.0"
        );
        assert_eq!(
            versioned_handoff_source("插件最终结果".into(), Page::Plugins),
            "插件最终结果 · plugins v1.15.0"
        );
        assert_eq!(
            versioned_handoff_source("MCP 工具结果".into(), Page::Mcp),
            "MCP 工具结果 · mcp-inspector v0.4.0"
        );
        assert_eq!(
            versioned_handoff_source("其他结果".into(), Page::SmallTools),
            "其他结果"
        );
    }

    #[test]
    fn receiving_text_invalidates_empty_and_nonempty_success_without_running_target() {
        for previous in ["", "older target input"] {
            for same_tool in [false, true] {
                let mut tools = ToolState::default();
                tools.select(ToolKind::Base64);
                tools.input = previous.into();
                let result = tools.run(0);
                assert!(tools.apply_result(result));
                assert!(tools.has_result());
                if !same_tool {
                    tools.select(ToolKind::Json);
                    tools.input = r#"{"source":true}"#.into();
                    let result = tools.run(0);
                    assert!(tools.apply_result(result));
                }
                let old_source = (tools.input.clone(), tools.output.clone());
                let mut transfer = Transfer::new("fixture".into(), "新的资料🦀").unwrap();
                transfer.target = Target::Tool(ToolKind::Base64);
                transfer
                    .apply(
                        &mut tools,
                        &mut DataState::default(),
                        &mut DiffState::default(),
                        &mut artifact_state(),
                    )
                    .unwrap();
                assert_eq!(tools.input, "新的资料🦀");
                assert!(
                    !tools.has_result(),
                    "new input has no completed result to copy or swap"
                );
                assert!(tools.output.is_empty() && tools.message.is_empty());
                assert_eq!(transfer.text, "新的资料🦀");
                if !same_tool {
                    tools.select(ToolKind::Json);
                    assert_eq!((tools.input.clone(), tools.output.clone()), old_source);
                    assert!(
                        tools.has_result(),
                        "unrelated source draft remains completed"
                    );
                }
                tools.select(ToolKind::Base64);
                let result = tools.run(0);
                assert!(tools.apply_result(result));
                assert_eq!(tools.output, "5paw55qE6LWE5paZ8J+mgA==");
                assert!(tools.has_result());
            }
        }
    }
    #[test]
    fn transfer_preserves_source_and_other_drafts_without_running_target() {
        let mut tools = ToolState::default();
        tools.input = "source".into();
        tools.output = "result".into();
        tools.select(ToolKind::Base64);
        tools.input = "old target".into();
        tools.output = "stale".into();
        tools.select(ToolKind::Json);
        let mut transfer = Transfer::new("JSON".into(), &tools.output).unwrap();
        transfer.target = Target::Tool(ToolKind::Base64);
        transfer
            .apply(
                &mut tools,
                &mut DataState::default(),
                &mut DiffState::default(),
                &mut artifact_state(),
            )
            .unwrap();
        assert_eq!(tools.input, "result");
        assert!(tools.output.is_empty());
        tools.select(ToolKind::Json);
        assert_eq!(tools.input, "source");
        assert_eq!(tools.output, "result");
    }

    #[test]
    fn artifact_handoff_opens_an_unpersisted_draft() {
        let path = std::env::temp_dir().join(format!(
            "zi-handoff-artifact-{}.sqlite3",
            uuid::Uuid::new_v4()
        ));
        let mut artifacts = crate::artifact_hub::State::new(path.clone());
        let mut transfer = Transfer::new("普通工具结果".into(), "临时内容").unwrap();
        transfer.target = Target::ArtifactHub;
        let (page, kind) = transfer
            .apply(
                &mut ToolState::default(),
                &mut DataState::default(),
                &mut DiffState::default(),
                &mut artifacts,
            )
            .unwrap();
        assert_eq!(page, Page::Artifacts);
        assert_eq!(kind, None);
        assert!(
            !path.exists(),
            "receiving a handoff must never persist content"
        );
    }

    #[test]
    fn structured_table_handoff_keeps_table_kind_in_artifact_draft() {
        let path = std::env::temp_dir().join(format!(
            "zi-handoff-table-artifact-{}.sqlite3",
            uuid::Uuid::new_v4()
        ));
        let dataset = crate::workbench::Dataset {
            headers: vec!["count".into()],
            rows: vec![vec![serde_json::json!(7)]].into(),
        };
        let mut transfer = Transfer::material(
            "pipeline v0.20.0 · 第1步结果".into(),
            Material::Table(dataset.clone()),
        )
        .unwrap();
        transfer.target = Target::ArtifactHub;
        assert_eq!(transfer.artifact_kind(), crate::artifact_hub::Kind::Table);
        let mut artifacts = crate::artifact_hub::State::new(path.clone());
        let (page, kind) = transfer
            .apply(
                &mut ToolState::default(),
                &mut DataState::default(),
                &mut DiffState::default(),
                &mut artifacts,
            )
            .unwrap();
        assert_eq!(page, Page::Artifacts);
        assert_eq!(kind, None);
        assert!(!path.exists(), "typed table receive must remain a draft");
        let (_, draft) = artifacts.handoff_snapshot().unwrap();
        assert_eq!(
            serde_json::from_str::<crate::workbench::Dataset>(&draft).unwrap(),
            dataset
        );
    }

    #[test]
    fn image_result_handoff_creates_a_new_text_flow_without_encoding_or_saving_pixels() {
        let image = crate::image_tools::ImageMaterial {
            image: std::sync::Arc::new(image::DynamicImage::new_rgba8(4, 3)),
            origins: Vec::new(),
            kind: "裁剪结果",
        };
        let mut transfer =
            Transfer::material("图片流程 v1 · 裁剪结果".into(), Material::Image(image)).unwrap();
        assert_eq!(transfer.target, Target::TextFlow);
        assert!(transfer.compatible());
        assert_eq!(transfer.recommendations[0].target, Target::TextFlow);
        assert!(!transfer.accepts_target(Target::JsonData));
        assert!(transfer.preview.contains("4 × 3"));
        assert!(!transfer.text.contains("data:image"));

        let mut data = crate::workbench::sessions::Workspace::new(
            std::env::temp_dir().join(format!("zi-handoff-image-{}.sqlite3", uuid::Uuid::new_v4())),
        );
        let before = data.instances.len();
        let (page, kind) = transfer.apply_image(&mut data).unwrap();
        assert_eq!(page, Page::Data);
        assert_eq!(kind, None);
        assert_eq!(data.instances.len(), before + 1);
        assert_eq!(data.active_tool_id(), "text-flow");
        assert!(
            !data.instances[1].state.busy(),
            "receiving an image must not run the workflow"
        );

        transfer.target = Target::ArtifactHub;
        assert!(transfer.compatible());
        assert!(transfer.matching_targets().contains(&Target::ArtifactHub));
    }

    #[test]
    fn artifact_table_snapshot_returns_to_data_workbench_as_typed_table() {
        let dataset = crate::workbench::Dataset {
            headers: vec!["value".into()],
            rows: vec![vec![serde_json::json!(7)]].into(),
        };
        let text = serde_json::to_string(&dataset).unwrap();
        let transfer = Transfer::artifact_snapshot(
            "本机资料「结果」· v1 · 来源 pipeline@0.20.0".into(),
            text,
            Some(dataset.clone()),
        )
        .unwrap();
        assert_eq!(transfer.target, Target::JsonData);
        assert!(matches!(transfer.material, Some(Material::Table(_))));

        let mut data = crate::workbench::sessions::Workspace::new(std::path::PathBuf::from(
            "unused-artifact-table-handoff.db",
        ));
        transfer.apply_material(&mut data).unwrap();
        assert_eq!(data.instances.len(), 2);
        assert_eq!(data.instances[1].name, transfer.source);
        assert_eq!(
            serde_json::from_str::<crate::workbench::Dataset>(&data.input).unwrap(),
            dataset
        );
    }

    #[test]
    fn numeric_typed_material_can_fill_calculator_without_loss_or_source_mutation() {
        let dataset = crate::workbench::Dataset {
            headers: vec!["fraction".into(), "count".into()],
            rows: vec![vec![
                serde_json::json!({"kind":"exact","numerator":"1","denominator":"3"}),
                serde_json::json!(7),
            ]]
            .into(),
        };
        let original = serde_json::to_string(&dataset).unwrap();
        let mut transfer = Transfer::material(
            "pipeline v0.20.0 · 数值结果".into(),
            Material::Table(dataset.clone()),
        )
        .unwrap();
        assert_eq!(transfer.recommendations[0].target, Target::Calculator);
        assert!(transfer.matching_targets().contains(&Target::Calculator));
        transfer.target = Target::Calculator;
        assert!(transfer.compatible());
        transfer.refresh_numeric();
        assert_eq!(
            NumericTable::read_json(&transfer.text).unwrap().cells,
            vec![
                crate::calculator::Value::Exact(1, 3),
                crate::calculator::Value::Exact(7, 1)
            ]
        );

        transfer.target = Target::ArtifactHub;
        transfer.refresh_numeric();
        assert_eq!(
            serde_json::from_str::<crate::workbench::Dataset>(&transfer.text).unwrap(),
            dataset
        );
        assert_eq!(serde_json::to_string(&dataset).unwrap(), original);
    }

    #[test]
    fn nonnumeric_or_oversized_material_is_not_offered_to_calculator() {
        for dataset in [
            crate::workbench::Dataset {
                headers: vec!["name".into()],
                rows: vec![vec![serde_json::json!("item")]].into(),
            },
            crate::workbench::Dataset {
                headers: vec!["count".into()],
                rows: (0..9).map(|n| vec![serde_json::json!(n)]).collect(),
            },
        ] {
            let mut transfer =
                Transfer::material("pipeline test".into(), Material::Table(dataset)).unwrap();
            assert!(!transfer.matching_targets().contains(&Target::Calculator));
            transfer.target = Target::Calculator;
            assert!(!transfer.compatible());
        }
    }
    #[test]
    fn diff_transfer_preserves_other_side_and_clears_stale_report() {
        let mut diff = DiffState {
            diff_before: "left".into(),
            diff_after: "right".into(),
            diff_output: "old".into(),
            message: "old error".into(),
        };
        let mut transfer = Transfer::new("report".into(), "new").unwrap();
        transfer.target = Target::After;
        transfer
            .apply(
                &mut ToolState::default(),
                &mut DataState::default(),
                &mut diff,
                &mut artifact_state(),
            )
            .unwrap();
        assert_eq!(diff.diff_before, "left");
        assert_eq!(diff.diff_after, "new");
        assert!(diff.diff_output.is_empty() && diff.message.is_empty());
    }
    #[test]
    fn busy_data_rejects_transfer_without_replacing_input() {
        let mut data = DataState::default();
        data.import_text(
            "id\noriginal".into(),
            crate::workbench::DataFormat::Csv,
            false,
        )
        .unwrap();
        let mut transfer = Transfer::new("result".into(), "id\nreplacement").unwrap();
        transfer.target = Target::Csv;
        assert!(
            transfer
                .apply(
                    &mut ToolState::default(),
                    &mut data,
                    &mut DiffState::default(),
                    &mut artifact_state(),
                )
                .is_err()
        );
        assert_eq!(data.input, "id\noriginal");
        assert_eq!(transfer.text, "id\nreplacement");
    }
    #[test]
    fn snapshot_is_bounded_and_preview_preserves_unicode() {
        assert!(Transfer::new("test".into(), "").is_err());
        assert!(Transfer::new("test".into(), &"a".repeat(2 * 1024 * 1024 + 1)).is_err());
        let text = "中文🦀".repeat(600);
        let transfer = Transfer::new("test".into(), &text).unwrap();
        assert_eq!(transfer.preview.chars().count(), 1200);
        assert_eq!(transfer.text, text);
    }

    #[test]
    fn typed_table_handoff_preserves_cells_and_opens_a_new_data_instance() {
        let dataset = crate::workbench::Dataset {
            headers: vec!["name".into(), "count".into(), "enabled".into()],
            rows: vec![vec![
                serde_json::json!("Zi"),
                serde_json::json!(7),
                serde_json::json!(true),
            ]]
            .into(),
        };
        let source = "pipeline v0.20.0 · 第1步结果";
        let mut transfer =
            Transfer::material(source.into(), Material::Table(dataset.clone())).unwrap();
        assert_eq!(transfer.source, source);
        assert_eq!(transfer.target, Target::JsonData);
        assert!(transfer.compatible());
        assert_eq!(
            serde_json::from_str::<crate::workbench::Dataset>(&transfer.text).unwrap(),
            dataset
        );
        assert!(transfer.preview.contains("Zi"));
        transfer.target = Target::Csv;
        assert!(!transfer.compatible());
        assert!(
            transfer
                .matching_targets()
                .iter()
                .all(|target| matches!(target, Target::JsonData | Target::ArtifactHub))
        );
        transfer.target = Target::JsonData;

        let mut data = crate::workbench::sessions::Workspace::new(std::path::PathBuf::from(
            "unused-material-handoff.db",
        ));
        data.import_text("old\nwork".into(), crate::workbench::DataFormat::Csv, false)
            .unwrap();
        let original_id = data.active_id().to_owned();
        transfer.apply_material(&mut data).unwrap();
        assert_eq!(data.instances.len(), 2);
        assert_ne!(data.active_id(), original_id);
        assert_eq!(data.instances[1].name, source);
        assert_eq!(data.input, transfer.text);
        assert_eq!(
            serde_json::from_str::<crate::workbench::Dataset>(&data.input).unwrap(),
            dataset
        );
        assert_eq!(data.instances[0].state.input, "old\nwork");
    }

    #[test]
    fn file_reference_handoff_shows_no_path_and_requires_explicit_encoding_read() {
        let folder = std::env::temp_dir().join(format!("zi-file-handoff-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&folder).unwrap();
        let path = folder.join("source.txt");
        std::fs::write(&path, b"private source text").unwrap();
        let material = crate::material_files::FileMaterial::selected(&path, 1024).unwrap();
        let mut transfer = Transfer::file("文件校验 · 原文件".into(), material.clone()).unwrap();
        assert_eq!(transfer.target, Target::FileEncoding);
        assert!(transfer.compatible());
        assert!(transfer.preview.contains("source.txt"));
        assert!(!transfer.preview.contains(folder.to_string_lossy().as_ref()));
        let compatible_targets = transfer.matching_targets();
        assert!(compatible_targets.contains(&Target::FileEncoding));
        assert!(compatible_targets.contains(&Target::TextFlow));
        transfer.target = Target::TextFlow;
        assert!(transfer.compatible());
        transfer.target = Target::JsonData;
        assert!(!transfer.compatible());
        transfer.target = Target::FileEncoding;

        let mut state = crate::file_encoding::State::default();
        let (page, _) = transfer.apply_file(&mut state).unwrap();
        assert_eq!(page, Page::FileEncoding);
        assert!(std::fs::read(&path).unwrap() == b"private source text");
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(&folder).unwrap();
    }
}
