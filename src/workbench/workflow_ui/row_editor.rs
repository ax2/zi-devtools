use super::*;

fn column(ui: &mut egui::Ui, name: &mut String, available: &[String]) -> bool {
    let mut changed = ui
        .add(
            egui::TextEdit::singleline(name)
                .char_limit(256)
                .hint_text("填写列名")
                .desired_width(140.0),
        )
        .changed();
    if !available.is_empty() {
        egui::ComboBox::from_id_salt("column")
            .selected_text("选择列")
            .show_ui(ui, |ui| {
                for candidate in available {
                    changed |= ui
                        .selectable_value(name, candidate.clone(), candidate)
                        .changed();
                }
            });
        if !available.contains(name) {
            ui.colored_label(ui.visuals().error_fg_color, "当前表缺少该列");
        }
    }
    changed
}

pub(super) fn edit(ui: &mut egui::Ui, step: &mut Step, available: &[String]) -> bool {
    let mut changed = false;
    match step {
        Step::Filter {
            column: name,
            predicate,
            value,
            case_sensitive,
        } => {
            ui.horizontal_wrapped(|ui| {
                ui.label("筛选列");
                changed |= column(ui, name, available);
                let previous = *predicate;
                egui::ComboBox::from_id_salt("predicate")
                    .selected_text(predicate.label())
                    .show_ui(ui, |ui| {
                        for p in [
                            Predicate::Contains,
                            Predicate::NotContains,
                            Predicate::Equals,
                            Predicate::NotEquals,
                            Predicate::IsNull,
                            Predicate::IsNotNull,
                        ] {
                            changed |= ui.selectable_value(predicate, p, p.label()).changed();
                        }
                    });
                if previous != *predicate
                    && matches!(predicate, Predicate::IsNull | Predicate::IsNotNull)
                {
                    value.clear();
                }
            });
            if !matches!(predicate, Predicate::IsNull | Predicate::IsNotNull) {
                ui.horizontal_wrapped(|ui| {
                    changed |= ui
                        .add(
                            egui::TextEdit::singleline(value)
                                .char_limit(4096)
                                .desired_width(250.0)
                                .hint_text(
                                    if matches!(predicate, Predicate::Equals | Predicate::NotEquals)
                                    {
                                        "JSON值，例如 1、\"001\"、true"
                                    } else {
                                        "查找文本"
                                    },
                                ),
                        )
                        .changed();
                    if matches!(predicate, Predicate::Contains | Predicate::NotContains) {
                        changed |= ui.checkbox(case_sensitive, "区分大小写").changed();
                    }
                });
            }
            ui.small("文本包含按显示值匹配；JSON相等区分类型。CSV单元格默认是字符串。");
        }
        Step::Sort { keys } => {
            ui.label("多列排序 · 上面的列优先，同值保留原顺序");
            let mut remove = None;
            let mut movement = None;
            let count = keys.len();
            for (index, key) in keys.iter_mut().enumerate() {
                ui.push_id(index, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(format!("优先级{}", index + 1));
                        changed |= column(ui, &mut key.column, available);
                        changed |= ui.checkbox(&mut key.descending, "降序").changed();
                        if ui
                            .add_enabled(index > 0, egui::Button::new("↑"))
                            .on_hover_text("提高优先级")
                            .clicked()
                        {
                            movement = Some((index, index - 1));
                        }
                        if ui
                            .add_enabled(index + 1 < count, egui::Button::new("↓"))
                            .on_hover_text("降低优先级")
                            .clicked()
                        {
                            movement = Some((index, index + 1));
                        }
                        if ui.button("移除").clicked() {
                            remove = Some(index);
                        }
                    });
                });
            }
            if let Some(index) = remove {
                keys.remove(index);
                changed = true;
            } else if let Some((a, b)) = movement {
                keys.swap(a, b);
                changed = true;
            }
            if ui
                .add_enabled(keys.len() < 4, egui::Button::new("添加排序列"))
                .clicked()
            {
                keys.push(SortKey {
                    column: available
                        .iter()
                        .find(|c| !keys.iter().any(|k| &k.column == *c))
                        .cloned()
                        .unwrap_or_default(),
                    descending: false,
                });
                changed = true;
            }
            ui.small("升序类型顺序：null、布尔、数字、文本、数组、对象；文本按Unicode排序。");
        }
        Step::Deduplicate { columns } => {
            ui.label("按所选列组合去重，保留当前顺序的首条");
            let mut remove = None;
            for (index, name) in columns.iter_mut().enumerate() {
                ui.push_id(index, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        changed |= column(ui, name, available);
                        if ui.button("移除").clicked() {
                            remove = Some(index);
                        }
                    });
                });
            }
            if let Some(index) = remove {
                columns.remove(index);
                changed = true;
            }
            if ui
                .add_enabled(columns.len() < 128, egui::Button::new("添加去重列"))
                .clicked()
            {
                columns.push(
                    available
                        .iter()
                        .find(|c| !columns.contains(c))
                        .cloned()
                        .unwrap_or_default(),
                );
                changed = true;
            }
            ui.small("按JSON表示区分值：数字1、数字1.0、文本1各自保留，其他列保持完整。");
        }
        _ => {}
    }
    changed
}

pub(super) fn review(ui: &mut egui::Ui, step: &Step) {
    match step {
        Step::Filter {
            column,
            predicate,
            value,
            case_sensitive,
        } => {
            ui.label(format!("筛选：{column} · {} · {value}", predicate.label()));
            if matches!(predicate, Predicate::Contains | Predicate::NotContains) {
                ui.label(if *case_sensitive {
                    "区分大小写"
                } else {
                    "不区分大小写"
                });
            }
        }
        Step::Sort { keys } => {
            ui.label(format!(
                "稳定排序：{}",
                keys.iter()
                    .map(|k| format!(
                        "{} {}",
                        k.column,
                        if k.descending { "降序" } else { "升序" }
                    ))
                    .collect::<Vec<_>>()
                    .join("、")
            ));
        }
        Step::Deduplicate { columns } => {
            ui.label(format!("去重保留首条：{}", columns.join("、")));
        }
        _ => {}
    }
}
