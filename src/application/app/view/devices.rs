//! Shared device-list presentation. Selection and actions always use device IDs.
use super::*;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Filter {
    #[default]
    All,
    Online,
    Offline,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Sort {
    #[default]
    Status,
    Name,
}

impl Sort {
    fn label(self) -> &'static str {
        match self {
            Self::Status => "在线优先",
            Self::Name => "名称排序",
        }
    }
}

#[derive(Default)]
pub(super) struct ListUi {
    search: String,
    filter: Filter,
    sort: Sort,
}

struct Entry {
    device: DeviceInfo,
    group: usize,
    current: bool,
    viewing: bool,
    own_session: bool,
    connect_issue: Option<String>,
    takeover: bool,
    status: crate::application::app::device_status::DeviceStatus,
}

enum Action {
    Details,
    Connect,
    Audio,
}

impl DeviceCenterApp {
    pub(super) fn devices_page(&mut self, ui: &mut egui::Ui) {
        self.device_list(ui, false);
    }
    pub(super) fn management_page(&mut self, ui: &mut egui::Ui) {
        self.device_list(ui, true);
    }

    fn device_entries(&self, management: bool) -> Option<Vec<Entry>> {
        let mut devices = Vec::new();
        if management {
            let catalog = self.catalog.as_ref()?;
            devices.extend(catalog.groups.desktop_devices.iter().map(|d| (d, 0)));
            devices.extend(catalog.groups.mobile_devices.iter().map(|d| (d, 1)));
            devices.extend(catalog.groups.tv_devices.iter().map(|d| (d, 2)));
        } else {
            devices.extend(
                all_devices(self.devices.as_ref()?)
                    .filter(|(_, d)| self.show_in_watching_list(d))
                    .map(|(_, d)| (d, 0)),
            );
        }
        Some(
            devices
                .into_iter()
                .map(|(device, group)| {
                    let id = device.device_id.as_str();
                    let own_session = self.viewer_for_device(id).is_some();
                    let connect_issue = self.viewer_action_issue(device);
                    Entry {
                        device: device.clone(),
                        group,
                        current: self
                            .catalog
                            .as_ref()
                            .is_some_and(|c| c.groups.current_device_id == id),
                        viewing: !management && self.is_viewing_target(id),
                        own_session,
                        connect_issue,
                        takeover: self.needs_takeover(device),
                        status: self.device_status(device),
                    }
                })
                .collect(),
        )
    }

    fn device_list(&mut self, ui: &mut egui::Ui, management: bool) {
        let rows = self.device_entries(management);
        let index = usize::from(management);
        let count = rows.as_ref().map_or(0, Vec::len);
        let online = rows
            .as_ref()
            .map_or(0, |r| r.iter().filter(|e| e.device.is_connected()).count());
        let offline = rows.as_ref().map_or(0, |r| {
            r.iter()
                .filter(|e| e.device.status == "DISCONNECTED")
                .count()
        });
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                crate::ui::controls::page_heading(
                    ui,
                    if management {
                        "设备目录"
                    } else {
                        "连接设备"
                    },
                    if management {
                        "管理账号中的电脑、移动设备和电视"
                    } else {
                        "选择一台设备，继续你的工作"
                    },
                );
            });
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                let response = ui.add_enabled(
                    !self.refresh_pending && !self.logout_pending,
                    egui::Button::new(if self.refresh_pending {
                        "刷新中…"
                    } else {
                        "刷新设备"
                    }),
                );
                if response.clicked() {
                    self.request_refresh();
                }
                if let Some(at) = self.refreshed_at {
                    response
                        .on_hover_text(format!("上次状态更新于 {} 秒前", at.elapsed().as_secs()));
                }
                if !management {
                    self.active_view(ui);
                }
            });
        });
        ui.add_space(24.0);
        let state = &mut self.center_ui.device_lists[index];
        ui.horizontal(|ui| {
            let width = (ui.available_width() - 182.0).max(140.0);
            ui.add_sized(
                [width, 36.0],
                singleline_input(&mut state.search).hint_text("查找设备 / 名称、ID、系统"),
            );
            if ui
                .add_enabled(!state.search.is_empty(), egui::Button::new("清除"))
                .clicked()
            {
                state.search.clear();
            }
            egui::ComboBox::from_id_salt(("device-sort", index))
                .width(108.0)
                .selected_text(state.sort.label())
                .show_ui(ui, |ui| {
                    for sort in [Sort::Status, Sort::Name] {
                        ui.selectable_value(&mut state.sort, sort, sort.label());
                    }
                });
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            for (filter, label, n) in [
                (Filter::All, "全部", count),
                (Filter::Online, "在线", online),
                (Filter::Offline, "离线", offline),
            ] {
                if crate::ui::controls::workspace_tab(
                    ui,
                    &format!("{label}  {n}"),
                    state.filter == filter,
                )
                .clicked()
                {
                    state.filter = filter;
                }
            }
        });
        let query = state.search.trim().to_lowercase();
        let filter = state.filter;
        let sort = state.sort;
        ui.add_space(16.0);
        self.alert(ui);
        crate::ui::controls::observe_notice(
            ui.ctx(),
            "catalog-error",
            "设备清单未更新",
            crate::ui::controls::DialogIcon::Warning,
            self.catalog_error.as_deref().filter(|_| management),
        );
        let Some(mut rows) = rows else {
            self.empty_state(ui, "正在读取设备清单…", None);
            return;
        };
        rows.retain(|e| {
            let d = &e.device;
            let visible = match filter {
                Filter::All => true,
                Filter::Online => d.is_connected(),
                Filter::Offline => d.status == "DISCONNECTED",
            };
            visible
                && (query.is_empty()
                    || [
                        display_alias(d),
                        &d.device_id,
                        &d.platform_label(),
                        &d.version_name,
                    ]
                    .iter()
                    .any(|value| value.to_lowercase().contains(&query)))
        });
        rows.sort_by(|a, b| {
            a.group
                .cmp(&b.group)
                .then_with(|| {
                    if sort == Sort::Status {
                        b.device.is_connected().cmp(&a.device.is_connected())
                    } else {
                        std::cmp::Ordering::Equal
                    }
                })
                .then_with(|| {
                    display_alias(&a.device)
                        .to_lowercase()
                        .cmp(&display_alias(&b.device).to_lowercase())
                })
                .then_with(|| a.device.device_id.cmp(&b.device.device_id))
        });
        if rows.is_empty() {
            let clear_filters = self.empty_state(
                ui,
                if count == 0 {
                    "暂无设备"
                } else {
                    "没有符合条件的设备"
                },
                (count > 0).then_some("清除搜索和筛选"),
            );
            if clear_filters {
                let state = &mut self.center_ui.device_lists[index];
                state.search.clear();
                state.filter = Filter::All;
            }
            return;
        }
        let mut picked = None;
        crate::ui::controls::page_scroll(("device-list", index)).show(ui, |ui| {
            let groups = ["电脑", "手机 / 平板", "电视"];
            for (group, title) in groups.iter().enumerate() {
                let entries = rows.iter().filter(|e| e.group == group).collect::<Vec<_>>();
                if entries.is_empty() {
                    continue;
                }
                if management {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(*title)
                                .strong()
                                .color(crate::ui::theme::color(MUTED)),
                        );
                        ui.label(
                            RichText::new(entries.len().to_string())
                                .small()
                                .color(crate::ui::theme::color(MUTED)),
                        );
                    });
                    ui.add_space(6.0);
                }
                for entry in entries {
                    let selected =
                        self.selected_device_id.as_deref() == Some(&entry.device.device_id);
                    if let Some(action) = ui
                        .push_id(&entry.device.device_id, |ui| {
                            row(ui, entry, selected, &mut self.center_ui.wallpapers)
                        })
                        .inner
                    {
                        picked = Some((entry.device.device_id.clone(), action));
                    }
                    ui.add_space(0.0);
                }
                ui.add_space(12.0);
            }
        });
        if let Some((id, action)) = picked {
            match action {
                Action::Details => self.open_details(id),
                Action::Connect => {
                    self.selected_device_id = Some(id);
                    self.start_viewer();
                }
                Action::Audio => {
                    self.selected_device_id = Some(id);
                    self.start_viewer_mode(true);
                }
            }
        }
    }
}

fn row(
    ui: &mut egui::Ui,
    entry: &Entry,
    selected: bool,
    _wallpapers: &mut crate::application::wallpaper::Wallpapers,
) -> Option<Action> {
    let device = &entry.device;
    let wide = ui.available_width() >= 760.0;
    let (rect, response) = ui.allocate_exact_size(
        vec2(ui.available_width(), if wide { 76.0 } else { 100.0 }),
        Sense::click(),
    );
    if !ui.is_rect_visible(rect) {
        return None;
    }
    let p = ui.painter();
    if selected || response.hovered() || response.has_focus() {
        p.rect_filled(
            rect,
            0.0,
            theme::color(if selected {
                theme::SELECTED
            } else {
                theme::HOVER
            }),
        );
    }
    p.hline(
        rect.x_range(),
        rect.bottom(),
        Stroke::new(1.0, theme::color(LINE)),
    );
    if selected || response.has_focus() {
        p.rect_filled(
            egui::Rect::from_min_size(rect.min, vec2(3.0, rect.height())),
            0.0,
            theme::color(BLUE),
        );
    }
    let actions_x = rect.right() - if entry.viewing { 210.0 } else { 92.0 };
    let status_x = if wide {
        actions_x - 120.0
    } else {
        rect.left() + 20.0
    };
    let metadata_x = if wide {
        status_x - 154.0
    } else {
        rect.left() + 180.0
    };
    let name_right = if wide {
        metadata_x - 16.0
    } else {
        actions_x - 12.0
    };
    let mut name = ui.new_child(egui::UiBuilder::new().max_rect(egui::Rect::from_min_max(
        rect.min + vec2(20.0, 14.0),
        egui::pos2(name_right, rect.top() + 64.0),
    )));
    name.add(
        egui::Label::new(
            RichText::new(display_alias(device))
                .size(crate::ui::theme::SECTION)
                .strong(),
        )
        .truncate(),
    )
    .on_hover_text(display_alias(device));
    name.add(
        egui::Label::new(
            RichText::new(format!(
                "{}{}",
                if entry.current { "本机 · " } else { "ID  " },
                device.device_id
            ))
            .size(crate::ui::theme::TINY)
            .color(crate::ui::theme::color(MUTED)),
        )
        .truncate(),
    )
    .on_hover_text(&device.device_id);
    let status_rect = egui::Rect::from_min_size(
        egui::pos2(status_x, rect.top() + if wide { 25.0 } else { 66.0 }),
        crate::ui::theme::DEVICE_STATUS_SIZE.into(),
    );
    crate::ui::controls::device_status_badge(
        ui,
        status_rect,
        entry.status.label(),
        entry.status.color(),
    );
    let mut metadata = ui.new_child(egui::UiBuilder::new().max_rect(egui::Rect::from_min_max(
        egui::pos2(metadata_x, rect.top() + if wide { 18.0 } else { 72.0 }),
        egui::pos2(
            if wide {
                status_x - 12.0
            } else {
                rect.right() - 14.0
            },
            rect.bottom() - 8.0,
        ),
    )));
    if wide {
        metadata.label(RichText::new(device.platform_label()).size(crate::ui::theme::COMPACT_TEXT));
        metadata.add(
            egui::Label::new(
                RichText::new(if device.version_name.is_empty() {
                    "版本未知"
                } else {
                    &device.version_name
                })
                .size(crate::ui::theme::TINY)
                .color(crate::ui::theme::color(MUTED)),
            )
            .truncate(),
        );
    } else {
        metadata.add(
            egui::Label::new(
                RichText::new(format!(
                    "{} · {}",
                    device.platform_label(),
                    if device.version_name.is_empty() {
                        "版本未知"
                    } else {
                        &device.version_name
                    }
                ))
                .size(crate::ui::theme::TINY)
                .color(crate::ui::theme::color(MUTED)),
            )
            .truncate(),
        );
    }
    let mut action = None;
    let mut buttons = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(egui::Rect::from_min_max(
                egui::pos2(actions_x, rect.top() + 12.0),
                egui::pos2(
                    rect.right() - 14.0,
                    rect.top() + if wide { 64.0 } else { 64.0 },
                ),
            ))
            .layout(egui::Layout::right_to_left(Align::Center)),
    );
    if entry.viewing {
        let button = buttons.add_enabled(
            entry.connect_issue.is_none(),
            primary(if entry.own_session {
                "已打开"
            } else if entry.takeover {
                "接管"
            } else {
                "连接"
            }),
        );
        if button.clicked() {
            action = Some(Action::Connect);
        }
        let audio = buttons
            .add_enabled_ui(entry.connect_issue.is_none(), |ui| {
                crate::ui::controls::audio_output_button(
                    ui,
                    "仅音频连接",
                    crate::ui::theme::CONTROL_HEIGHT,
                )
            })
            .inner;
        if audio.clicked() {
            action = Some(Action::Audio);
        }
        if let Some(issue) = &entry.connect_issue {
            audio.on_disabled_hover_text(issue);
        }
        if let Some(issue) = &entry.connect_issue {
            button.on_disabled_hover_text(issue);
        }
    }
    if buttons.button("详情").clicked() {
        action = Some(Action::Details);
    }
    action.or_else(|| response.clicked().then_some(Action::Details))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(blocked: bool) -> Entry {
        let device: DeviceInfo = serde_json::from_value(serde_json::json!({
            "device_id": "0123456789abcdef", "alias": "omarchy workstation",
            "status": "CONNECTED", "platform": 4,
            "controllable": true, "controlled_support": true
        }))
        .unwrap();
        let status = crate::application::app::device_status::connection_status(&device, "preview");
        Entry {
            device,
            group: 0,
            current: false,
            viewing: true,
            own_session: false,
            connect_issue: blocked.then(|| "连接不可用".into()),
            takeover: false,
            status,
        }
    }

    fn frame(
        ctx: &egui::Context,
        width: f32,
        entry: &Entry,
        events: Vec<egui::Event>,
    ) -> (egui::FullOutput, Option<Action>) {
        let mut action = None;
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    vec2(width, 180.0),
                )),
                events,
                ..Default::default()
            },
            |root| {
                egui::CentralPanel::default().show(root, |ui| {
                    action = row(ui, entry, false, &mut Default::default());
                });
            },
        );
        (output, action)
    }

    fn connect_center(output: &egui::FullOutput) -> egui::Pos2 {
        fn text_position(shape: &egui::Shape) -> Option<egui::Pos2> {
            match shape {
                egui::Shape::Text(text) if text.galley.job.text == "连接" => {
                    Some(text.pos + text.galley.size() * 0.5)
                }
                egui::Shape::Vec(shapes) => shapes.iter().find_map(text_position),
                _ => None,
            }
        }
        output
            .shapes
            .iter()
            .find_map(|s| text_position(&s.shape))
            .expect("connect action is visible")
    }

    #[test]
    fn compact_rows_keep_connect_actions_clickable_and_respect_refusals() {
        for width in [680.0, 780.0, 960.0] {
            for blocked in [false, true] {
                let ctx = egui::Context::default();
                crate::ui::fonts::install(&ctx);
                theme::configure(&ctx);
                let entry = sample(blocked);
                let (output, _) = frame(&ctx, width, &entry, vec![]);
                let pos = connect_center(&output);
                frame(
                    &ctx,
                    width,
                    &entry,
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed: true,
                            modifiers: Default::default(),
                        },
                    ],
                );
                let (_, action) = frame(
                    &ctx,
                    width,
                    &entry,
                    vec![egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: Default::default(),
                    }],
                );
                assert_eq!(
                    matches!(action, Some(Action::Connect)),
                    !blocked,
                    "width={width}, blocked={blocked}"
                );
            }
        }
    }
    #[test]
    #[ignore = "exports review PNGs; requires Vulkan and OMARCHYUU_REVIEW_DIR"]
    #[cfg(target_os = "linux")]
    fn render_omarchyuu_review() {
        let root = std::env::var_os("OMARCHYUU_REVIEW_DIR").expect("review directory");
        let root = std::path::PathBuf::from(root);
        std::fs::create_dir_all(&root).unwrap();
        for width in [960, 1080] {
            crate::ui::snapshots::render(
                &root.join(format!("devices-{width}.png")),
                [width, 740],
                |root_ui| {
                    super::super::review::workspace(root_ui, Page::Mine, |ui| {
                        crate::ui::controls::page_heading(
                            ui,
                            "连接设备",
                            "选择一台设备，继续你的工作",
                        );
                        ui.add_space(24.0);
                        let mut search = String::new();
                        ui.add_sized(
                            [ui.available_width(), 36.0],
                            singleline_input(&mut search).hint_text("查找设备 / 名称、ID、系统"),
                        );
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            for (label, selected) in
                                [("全部  3", true), ("在线  2", false), ("离线  1", false)]
                            {
                                crate::ui::controls::workspace_tab(ui, label, selected);
                            }
                        });
                        ui.add_space(16.0);
                        for (name, online) in [
                            ("Omarchy workstation", true),
                            ("Living room PC", true),
                            ("Travel laptop", false),
                        ] {
                            let mut entry = sample(!online);
                            entry.device.alias = name.into();
                            if !online {
                                entry.device.status = "DISCONNECTED".into();
                                entry.status =
                                    crate::application::app::device_status::connection_status(
                                        &entry.device,
                                        "preview",
                                    );
                            }
                            row(ui, &entry, false, &mut Default::default());
                        }
                    });
                },
            )
            .unwrap();
        }
    }
}
