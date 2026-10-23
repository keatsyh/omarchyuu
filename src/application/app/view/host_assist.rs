use super::*;
use crate::features::host::assist::{Action, Mode, Passwords, Settings};

pub(super) struct PasswordEditor {
    generation: u64,
    passwords: Passwords,
    code: String,
    confirmation: String,
    visible: bool,
}

struct CardAction {
    enabled: bool,
    settings: bool,
    refresh: bool,
    retry: bool,
}

fn share_text(id: &str, settings: &Settings) -> String {
    let mut text = format!(
        "我正在使用 omarchyuu，邀请你远程协助我的电脑。\n\n设备 ID：{}\n",
        assist_components::grouped_id(id)
    );
    if !settings.mode.needs_password() {
        text.push_str("无需验证码，连接时由我在本机确认。\n");
    } else {
        if settings.passwords == Passwords::Custom {
            text.push_str("已设置自定义密码，请单独向我获取。\n");
        } else {
            text.push_str(&format!("验证码：{}\n", settings.code));
        }
        if settings.mode.needs_confirmation() {
            text.push_str("本次连接需要密码验证和我的本机确认。\n");
        }
    }
    text.push_str("\n打开 omarchyuu 或网易 UU 远程，在“远程协助”中输入设备 ID 发起连接。");
    text
}

fn local_card(
    ui: &mut egui::Ui,
    snapshot: &crate::features::host::assist::Snapshot,
    allowed: bool,
    saving: bool,
    show_code: &mut bool,
    save_error: Option<&str>,
) -> CardAction {
    use super::assist_components::{copy_value, glyph, grouped_id};
    let settings = &snapshot.settings;
    let mut action = CardAction {
        enabled: settings.enabled,
        settings: false,
        refresh: false,
        retry: false,
    };
    crate::ui::controls::section_frame()
        .inner_margin(20)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("/ 让伙伴接入本机")
                        .size(theme::SECTION)
                        .strong(),
                );
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    ui.add_enabled_ui((allowed || action.enabled) && !saving, |ui| {
                        crate::ui::controls::switch(ui, &mut action.enabled);
                    });
                    ui.label(RichText::new("临时协助").small().color(theme::color(MUTED)));
                });
            });
            let enabled = allowed && action.enabled;
            if !enabled {
                *show_code = false;
            }
            let password = if settings.passwords == Passwords::Custom {
                &settings.custom_code
            } else {
                &settings.code
            };
            ui.add_space(20.0);
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(
                    vec2(theme::ASSIST_ID_WIDTH, 80.0),
                    egui::Layout::top_down(Align::Min),
                    |ui| {
                        ui.label(
                            RichText::new("本机设备 ID")
                                .small()
                                .color(theme::color(MUTED)),
                        );
                        copy_value(
                            ui,
                            "device-id",
                            vec2(theme::ASSIST_ID_WIDTH, 40.0),
                            RichText::new(if snapshot.connect_id.is_empty() {
                                "等待获取".into()
                            } else {
                                grouped_id(&snapshot.connect_id)
                            })
                            .monospace()
                            .strong()
                            .size(theme::TITLE),
                            (!snapshot.connect_id.is_empty())
                                .then_some(snapshot.connect_id.as_str()),
                            "已复制设备 ID",
                        );
                    },
                );
                ui.add_space(24.0);
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(if settings.mode.needs_password() {
                            "验证码 / 密码"
                        } else {
                            "连接方式"
                        })
                        .small()
                        .color(theme::color(MUTED)),
                    );
                    ui.horizontal(|ui| {
                        let code = if !settings.mode.needs_password() {
                            "由本机确认"
                        } else if enabled && *show_code {
                            password.as_str()
                        } else {
                            "••••••••"
                        };
                        let width =
                            (ui.available_width() - theme::CONTROL_HEIGHT * 2.0 - 24.0).max(100.0);
                        copy_value(
                            ui,
                            "password",
                            vec2(width, 40.0),
                            RichText::new(code).monospace().size(theme::DIALOG_TITLE),
                            (enabled && settings.mode.needs_password() && !password.is_empty())
                                .then_some(password.as_str()),
                            "已复制验证码",
                        );
                        if settings.mode.needs_password() {
                            ui.add_enabled_ui(enabled, |ui| {
                                crate::ui::controls::visibility_button(ui, show_code);
                            });
                        }
                        if enabled && settings.passwords.random() && settings.mode.needs_password()
                        {
                            action.refresh =
                                glyph(ui, Icon::Refresh, "更换验证码", theme::color(MUTED))
                                    .clicked();
                        }
                    });
                });
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(settings.mode.label())
                        .small()
                        .color(theme::color(MUTED)),
                );
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    action.settings = ui
                        .add_enabled(
                            allowed && !saving,
                            crate::ui::controls::secondary("验证设置"),
                        )
                        .clicked();
                    let copied_key = ui.id().with("assist-invitation-copied");
                    let fingerprint =
                        egui::Id::new((&snapshot.connect_id, password, settings.mode.wire()));
                    let copied = ui
                        .ctx()
                        .data(|d| d.get_temp::<(Instant, egui::Id)>(copied_key))
                        .filter(|(at, id)| {
                            *id == fingerprint && at.elapsed() < Duration::from_secs(2)
                        });
                    if ui
                        .add_enabled(
                            enabled && !snapshot.connect_id.is_empty(),
                            crate::ui::controls::secondary(if copied.is_some() {
                                "邀请已复制"
                            } else {
                                "复制协助邀请"
                            }),
                        )
                        .clicked()
                    {
                        ui.ctx()
                            .copy_text(share_text(&snapshot.connect_id, settings));
                        ui.ctx()
                            .data_mut(|d| d.insert_temp(copied_key, (Instant::now(), fingerprint)));
                    }
                    if let Some((at, _)) = copied {
                        ui.ctx().request_repaint_after(
                            Duration::from_secs(2).saturating_sub(at.elapsed()),
                        );
                    }
                });
            });
            if !allowed {
                ui.label(
                    RichText::new("请先在连接设置中开启“允许被控”")
                        .small()
                        .color(theme::color(MUTED)),
                );
            }
            if let Some(error) = save_error {
                ui.colored_label(theme::color(AMBER), format!("设置未保存：{error}"));
            }
            if let Some(error) = &snapshot.error {
                ui.horizontal(|ui| {
                    ui.colored_label(theme::color(AMBER), error);
                    action.retry = ui.add(crate::ui::controls::secondary("重试")).clicked();
                });
            }
        });
    action
}

impl DeviceCenterApp {
    fn edit_assist_password(&mut self, passwords: Passwords) {
        self.center_ui.host_assist_settings_open = false;
        self.center_ui.host_assist_password = Some(PasswordEditor {
            generation: self.login_generation,
            passwords,
            code: String::new(),
            confirmation: String::new(),
            visible: false,
        });
    }
    fn host_assist_action(&mut self, action: Action) {
        if self
            .worker
            .commands
            .send(GuiCommand::HostAssist {
                generation: self.login_generation,
                action,
            })
            .is_err()
        {
            self.status = StatusMessage::error("协助后台已停止");
        }
    }

    pub(super) fn host_assist_controls(&mut self, ui: &mut egui::Ui) {
        let Some(host) = self.host.clone() else {
            return;
        };
        let snapshot = host.assistance.snapshot();
        let action = local_card(
            ui,
            &snapshot,
            host.allowed() || host.is_guest(),
            host.status().saving,
            &mut self.center_ui.host_assist_show_code,
            host.status().settings_error.as_deref(),
        );
        let mut settings = snapshot.settings.clone();
        settings.enabled = action.enabled;
        if (settings.enabled && settings.code.is_empty()) || action.refresh {
            settings.refresh_code();
        }
        if settings != snapshot.settings {
            if host.is_guest() {
                host.set_allowed(settings.enabled);
            }
            match host.set_assistance(settings) {
                Ok(()) => self.save_host_settings(),
                Err(error) => self.status = StatusMessage::error(error.to_string()),
            }
        }
        if action.settings {
            self.center_ui.host_assist_settings_open = true;
        }
        if action.retry {
            self.host_assist_action(Action::Refresh);
        }
    }
    pub(super) fn host_assist_dialog(&mut self, ctx: &egui::Context) -> bool {
        if self.assist_password_dialog(ctx) {
            return true;
        }
        self.assist_settings_dialog(ctx)
    }

    fn assist_password_dialog(&mut self, ctx: &egui::Context) -> bool {
        let Some(mut editor) = self.center_ui.host_assist_password.take() else {
            return false;
        };
        if editor.generation != self.login_generation || self.logout_pending {
            return false;
        }
        let Some(host) = self.host.clone() else {
            return false;
        };
        let mut save = false;
        let mut cancel = false;
        let response = egui::Modal::new(egui::Id::new("host-assist-password-editor"))
            .frame(dialog_frame())
            .show(ctx, |ui| {
                ui.set_width(theme::MESSAGE_DIALOG_WIDTH);
                cancel = crate::ui::controls::dialog_header(
                    ui,
                    "自定义密码",
                    crate::ui::controls::DialogIcon::Edit,
                    true,
                );
                ui.label("8至16位字母和数字，必须同时包含两者");
                ui.add_space(12.0);
                ui.label("新密码");
                ui.add_sized(
                    [ui.available_width(), theme::CONTROL_HEIGHT],
                    singleline_input(&mut editor.code)
                        .password(!editor.visible)
                        .char_limit(128),
                );
                ui.label("再次输入");
                ui.add_sized(
                    [ui.available_width(), theme::CONTROL_HEIGHT],
                    singleline_input(&mut editor.confirmation)
                        .password(!editor.visible)
                        .char_limit(128),
                );
                ui.checkbox(&mut editor.visible, "显示密码");
                let valid = Settings::valid_custom_code(&editor.code);
                let matching = editor.code == editor.confirmation;
                if !editor.code.is_empty() && !valid {
                    ui.colored_label(crate::ui::theme::color(AMBER), "密码不符合格式要求");
                } else if !editor.confirmation.is_empty() && !matching {
                    ui.colored_label(crate::ui::theme::color(AMBER), "两次输入的密码不一致");
                }
                let (accept, dismiss) = crate::ui::controls::dialog_actions(
                    ui,
                    Some(
                        crate::ui::controls::DialogAction::new("保存并应用")
                            .enabled(valid && matching && !host.status().saving),
                    ),
                    Some("取消"),
                );
                save = accept;
                cancel |= dismiss;
            });
        if save {
            self.center_ui.host_assist_settings_open = true;
            self.center_ui.host_assist_show_code = false;
            let mut settings = host.assistance.settings();
            settings.custom_code = std::mem::take(&mut editor.code);
            settings.passwords = editor.passwords;
            if settings.mode == Mode::Confirmation {
                settings.mode = Mode::Password;
            }
            match host.set_assistance(settings) {
                Ok(()) => self.save_host_settings(),
                Err(error) => self.status = StatusMessage::error(error.to_string()),
            }
        } else if !cancel && !response.should_close() {
            self.center_ui.host_assist_password = Some(editor);
        } else {
            self.center_ui.host_assist_settings_open = true;
        }
        true
    }

    fn assist_settings_dialog(&mut self, ctx: &egui::Context) -> bool {
        if !self.center_ui.host_assist_settings_open {
            return false;
        }
        let Some(host) = self.host.clone() else {
            return false;
        };
        let original = host.assistance.settings();
        let mut settings = original.clone();
        let original_clipboard = host.clipboard_settings();
        let mut clipboard = original_clipboard;
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::new("host-assist-settings"))
            .frame(dialog_frame())
            .show(ctx, |ui| {
                ui.set_width(theme::MESSAGE_DIALOG_WIDTH);
                close = crate::ui::controls::dialog_header(
                    ui,
                    "远程协助设置",
                    crate::ui::controls::DialogIcon::Info,
                    true,
                );
                ui.add_enabled_ui(!host.status().saving, |ui| {
                    ui.label(RichText::new("验证方式").color(crate::ui::theme::color(MUTED)));
                    egui::ComboBox::from_id_salt("host-assist-mode")
                        .width(ui.available_width())
                        .selected_text(settings.mode.label())
                        .show_ui(ui, |ui| {
                            for mode in Mode::ALL {
                                ui.selectable_value(&mut settings.mode, mode, mode.label());
                            }
                        });
                    if settings.mode.needs_password() {
                        ui.add_space(12.0);
                        ui.label(RichText::new("密码类型").color(crate::ui::theme::color(MUTED)));
                        let mut selected = settings.passwords;
                        egui::ComboBox::from_id_salt("host-assist-passwords")
                            .width(ui.available_width())
                            .selected_text(selected.label())
                            .show_ui(ui, |ui| {
                                for choice in Passwords::ALL {
                                    ui.selectable_value(&mut selected, choice, choice.label());
                                }
                            });
                        if selected != settings.passwords {
                            if selected.custom() && settings.custom_code.is_empty() {
                                self.edit_assist_password(selected);
                            } else {
                                settings.passwords = selected;
                            }
                        }
                        ui.add_space(12.0);
                        ui.horizontal(|ui| {
                            ui.label("自定义密码");
                            ui.label(
                                RichText::new(if settings.custom_code.is_empty() {
                                    "未设置"
                                } else {
                                    "已设置"
                                })
                                .color(crate::ui::theme::color(MUTED)),
                            );
                            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                                if ui
                                    .add(crate::ui::controls::secondary(
                                        if settings.custom_code.is_empty() {
                                            "设置密码"
                                        } else {
                                            "修改密码"
                                        },
                                    ))
                                    .clicked()
                                {
                                    self.edit_assist_password(
                                        if settings.passwords == Passwords::Random {
                                            Passwords::Custom
                                        } else {
                                            settings.passwords
                                        },
                                    );
                                }
                            });
                        });
                    }
                });
                if host.is_guest() {
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        ui.label("允许剪贴板同步");
                        crate::ui::controls::switch(ui, &mut clipboard.enabled);
                    });
                    ui.add_enabled_ui(clipboard.enabled, |ui| {
                        ui.horizontal(|ui| {
                            ui.label("允许文件复制");
                            crate::ui::controls::switch(ui, &mut clipboard.files);
                        });
                    });
                }
                close |= crate::ui::controls::dialog_actions(
                    ui,
                    Some(crate::ui::controls::DialogAction::new("完成")),
                    None,
                )
                .0;
            });
        if clipboard != original_clipboard {
            match host.set_clipboard_settings(clipboard) {
                Ok(()) => self.save_host_settings(),
                Err(error) => self.status = StatusMessage::error(error.to_string()),
            }
        }
        if settings != original {
            self.center_ui.host_assist_show_code = false;
            match host.set_assistance(settings) {
                Ok(()) => self.save_host_settings(),
                Err(error) => self.status = StatusMessage::error(error.to_string()),
            }
        }
        if close || modal.should_close() {
            self.center_ui.host_assist_settings_open = false;
        }
        true
    }
}

#[cfg(test)]
pub(super) fn review_card(ui: &mut egui::Ui) {
    let snapshot = crate::features::host::assist::Snapshot {
        connect_id: "123456789".into(),
        settings: Settings {
            enabled: true,
            code: "sample123".into(),
            ..Default::default()
        },
        ready: true,
        ..Default::default()
    };
    local_card(ui, &snapshot, true, false, &mut false, None);
}
