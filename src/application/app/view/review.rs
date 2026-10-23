//! Opt-in previews use production widgets with fixture data, without accounts or sessions.
use super::*;

pub(super) fn workspace(root: &mut egui::Ui, page: Page, mut content: impl FnMut(&mut egui::Ui)) {
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE)
        .show(root, |root| {
            egui::Panel::top("review-caption")
                .exact_size(36.0)
                .frame(
                    egui::Frame::new()
                        .fill(theme::color(SIDEBAR))
                        .inner_margin(egui::Margin::symmetric(24, 6)),
                )
                .show(root, |ui| {
                    ui.label(RichText::new("omarchyuu / 远程工作区").monospace().small());
                });
            egui::Panel::top("review-workspaces")
                .exact_size(54.0)
                .frame(
                    egui::Frame::new()
                        .fill(theme::color(SIDEBAR))
                        .inner_margin(egui::Margin::symmetric(24, 8)),
                )
                .show(root, |ui| {
                    ui.horizontal(|ui| {
                        for (destination, icon, label) in [
                            (Page::Mine, Icon::Monitor, "设备"),
                            (Page::Assist, Icon::Assist, "协助"),
                            (Page::TerminalSessions, Icon::Terminal, "终端"),
                            (Page::Settings, Icon::Settings, "设置"),
                        ] {
                            nav_item(
                                ui,
                                icon,
                                label,
                                (destination == Page::Mine).then_some(3),
                                destination == page,
                            );
                        }
                        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                            ui.label(
                                RichText::new("远程工作区")
                                    .small()
                                    .color(theme::color(MUTED)),
                            );
                        });
                    });
                });
            egui::Panel::bottom("review-status")
                .exact_size(34.0)
                .frame(
                    egui::Frame::new()
                        .fill(theme::color(SIDEBAR))
                        .inner_margin(egui::Margin::symmetric(24, 6)),
                )
                .show(root, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("本机在线").small().color(theme::color(GREEN)));
                        ui.separator();
                        ui.label(RichText::new("示例账号").small());
                        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                            ui.label(RichText::new("v0.1.0").small());
                        });
                    });
                });
            if page == Page::Settings {
                egui::Panel::left("review-settings-index")
                    .exact_size(164.0)
                    .resizable(false)
                    .frame(
                        egui::Frame::new()
                            .fill(theme::color(BG))
                            .inner_margin(egui::Margin::symmetric(16, 24)),
                    )
                    .show(root, |ui| {
                        ui.label(RichText::new("偏好设置").small().color(theme::color(MUTED)));
                        ui.add_space(16.0);
                        for (index, label) in [
                            "连接与被控",
                            "快捷键",
                            #[cfg(windows)]
                            "插件",
                            "日志",
                            "诊断",
                            "关于",
                        ]
                        .iter()
                        .enumerate()
                        {
                            ui.add_sized(
                                [ui.available_width(), 36.0],
                                egui::Button::new(if index == 0 {
                                    format!("> {label}")
                                } else {
                                    format!("  {label}")
                                })
                                .fill(if index == 0 {
                                    theme::color(theme::SELECTED)
                                } else {
                                    Color32::TRANSPARENT
                                })
                                .stroke(Stroke::NONE),
                            );
                        }
                    });
            } else if matches!(page, Page::Mine | Page::Assist) {
                egui::Panel::top("review-scope")
                    .exact_size(46.0)
                    .frame(
                        egui::Frame::new()
                            .fill(theme::color(BG))
                            .inner_margin(egui::Margin::symmetric(24, 4)),
                    )
                    .show(root, |ui| {
                        ui.horizontal(|ui| {
                            for (index, label) in (if page == Page::Mine {
                                ["可连接", "账号中的全部设备"]
                            } else {
                                ["临时协助", "收藏"]
                            })
                            .iter()
                            .enumerate()
                            {
                                crate::ui::controls::workspace_tab(ui, label, index == 0);
                            }
                        });
                    });
            }
            egui::CentralPanel::default()
                .frame(
                    egui::Frame::new()
                        .fill(theme::color(BG))
                        .inner_margin(egui::Margin::symmetric(24, 20)),
                )
                .show(root, |ui| content(ui));
        });
}

#[test]
#[ignore = "exports fixture previews; requires Vulkan and OMARCHYUU_REVIEW_DIR"]
#[cfg(target_os = "linux")]
fn render_workspace_review() {
    let root = std::path::PathBuf::from(
        std::env::var_os("OMARCHYUU_REVIEW_DIR").expect("review directory"),
    );
    std::fs::create_dir_all(&root).unwrap();
    for width in [960, 1080] {
        for method in [LoginMethod::Qr, LoginMethod::Phone] {
            let name = if method == LoginMethod::Qr {
                "login-qr"
            } else {
                "login-phone"
            };
            crate::ui::snapshots::render(
                &root.join(format!("{name}-{width}.png")),
                [width, 740],
                |ui| {
                    ui.ctx().data_mut(|d| {
                        d.insert_temp(egui::Id::new("omarchyuu-login-method"), method)
                    });
                    let brand = crate::ui::branding::load_texture(ui.ctx());
                    login_surface(ui, &brand, |ui, method| match method {
                        LoginMethod::Qr => {
                            qr_form(ui, None, false, true, "", None);
                        }
                        LoginMethod::Phone => {
                            phone_form(ui, &mut Default::default(), false, false);
                        }
                    });
                },
            )
            .unwrap();
        }
        crate::ui::snapshots::render(
            &root.join(format!("settings-{width}.png")),
            [width, 740],
            |ui| {
                workspace(ui, Page::Settings, |ui| {
                    crate::ui::controls::page_heading(ui, "连接与被控", "");
                    section(ui, "本机访问");
                    form_row(ui, "开机自启", "登录桌面时启动 omarchyuu", |ui| {
                        crate::ui::controls::switch(ui, &mut true);
                    });
                    form_row(
                        ui,
                        "允许被控",
                        "使用原有账号与访问权限",
                        |ui| {
                            crate::ui::controls::switch(ui, &mut true);
                        },
                    );
                    form_row(ui, "访问权限", "已开启 8 / 8 项", |ui| {
                        ui.add(crate::ui::controls::secondary("展开"));
                    });
                    section(ui, "本机作为主控");
                    form_row(ui, "串流帧率", "以远端实际刷新率为准", |ui| {
                        egui::ComboBox::from_id_salt("review-fps")
                            .width(238.0)
                            .selected_text("跟随远端")
                            .show_ui(ui, |_| {});
                    });
                    form_row(
                        ui,
                        "编码格式",
                        "不可用时自动回退；重连后生效",
                        |ui| {
                            egui::ComboBox::from_id_salt("review-codec")
                                .width(238.0)
                                .selected_text("自动选择")
                                .show_ui(ui, |_| {});
                        },
                    );
                })
            },
        )
        .unwrap();
        crate::ui::snapshots::render(
            &root.join(format!("assist-{width}.png")),
            [width, 740],
            |ui| {
                workspace(ui, Page::Assist, |ui| {
                    crate::ui::controls::page_heading(ui, "远程协助", "");
                    ui.add_space(20.0);
                    host_assist::review_card(ui);
                    ui.add_space(24.0);
                    crate::ui::controls::section_frame().show(ui, |ui| {
                        assist_components::header(
                            ui,
                            "连接另一台设备",
                            Some("输入对方的设备 ID，发起一次协助"),
                            |_| {},
                        );
                        egui::Frame::new().inner_margin(20).show(ui, |ui| {
                            assist_components::connect_inputs(
                                ui,
                                &mut String::new(),
                                &mut String::new(),
                                true,
                            );
                            ui.add_space(16.0);
                            ui.label(RichText::new("最近连接").small().color(theme::color(MUTED)));
                            ui.label("连接过的设备会显示在这里。");
                        });
                    });
                })
            },
        )
        .unwrap();
        crate::ui::snapshots::render(
            &root.join(format!("controls-{width}.png")),
            [width, 740],
            |ui| {
                workspace(ui, Page::TerminalSessions, |ui| {
                    crate::ui::controls::page_heading(ui, "终端会话", "关闭连接后，会话继续运行。");
                    ui.add_space(24.0);
                    crate::ui::controls::section_frame()
                        .inner_margin(16)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.strong(">_ 工作会话");
                                ui.label(
                                    RichText::new("运行中").small().color(theme::color(GREEN)),
                                );
                                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                                    ui.add(primary("在本地打开"));
                                });
                            });
                            ui.label(
                                RichText::new("/usr/bin/bash    本机创建")
                                    .monospace()
                                    .small()
                                    .color(theme::color(MUTED)),
                            );
                        });
                    ui.add_space(24.0);
                    ui.columns(2, |cols| {
                        crate::ui::controls::section_frame().inner_margin(16).show(
                            &mut cols[0],
                            |ui| {
                                ui.strong("串流选项");
                                ui.add_space(12.0);
                                crate::ui::controls::menu_row(ui, "全屏", "", None, true, false);
                                crate::ui::controls::menu_row(ui, "画质", "自动", None, true, true);
                                crate::ui::controls::menu_row(
                                    ui,
                                    "剪贴板同步",
                                    "",
                                    Some(true),
                                    true,
                                    false,
                                );
                                crate::ui::controls::menu_row(
                                    ui,
                                    "麦克风",
                                    "",
                                    Some(false),
                                    true,
                                    false,
                                );
                            },
                        );
                        dialog_frame().show(&mut cols[1], |ui| {
                            crate::ui::controls::dialog_header(
                                ui,
                                "结束会话？",
                                crate::ui::controls::DialogIcon::Warning,
                                true,
                            );
                            ui.label("会话中的程序会退出，未保存的内容可能丢失。");
                            crate::ui::controls::dialog_actions(
                                ui,
                                Some(
                                    crate::ui::controls::DialogAction::new("结束会话").danger(true),
                                ),
                                Some("保留会话"),
                            );
                        });
                    });
                })
            },
        )
        .unwrap();
    }
}

#[test]
fn login_methods_can_be_switched_without_losing_the_phone_draft() {
    fn text_center(output: &egui::FullOutput, label: &str) -> egui::Pos2 {
        fn find(shape: &egui::Shape, label: &str) -> Option<egui::Pos2> {
            match shape {
                egui::Shape::Text(text) if text.galley.job.text == label => {
                    Some(text.pos + text.galley.size() * 0.5)
                }
                egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| find(s, label)),
                _ => None,
            }
        }
        output
            .shapes
            .iter()
            .find_map(|s| find(&s.shape, label))
            .expect("login method tab visible")
    }
    let ctx = egui::Context::default();
    crate::ui::fonts::install(&ctx);
    theme::configure(&ctx);
    let brand = crate::ui::branding::load_texture(&ctx);
    let mut phone = super::super::phone::PhoneForm::default();
    phone.mobile = "13800138000".into();
    let mut frame = |events| {
        let mut visible = Vec::new();
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    vec2(960.0, 680.0),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                login_surface(ui, &brand, |ui, method| {
                    visible.push(method);
                    match method {
                        LoginMethod::Qr => {
                            qr_form(ui, None, false, true, "", None);
                        }
                        LoginMethod::Phone => {
                            phone_form(ui, &mut phone, false, false);
                        }
                    }
                })
            },
        );
        (output, visible)
    };
    let (mut output, visible) = frame(vec![]);
    assert!(visible == [LoginMethod::Qr]);
    for (label, expected) in [
        ("短信登录", LoginMethod::Phone),
        ("扫码登录", LoginMethod::Qr),
        ("短信登录", LoginMethod::Phone),
    ] {
        let pos = text_center(&output, label);
        frame(vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            },
        ]);
        let (next, visible) = frame(vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Default::default(),
        }]);
        assert!(visible == [expected]);
        output = next;
    }
    assert_eq!(phone.mobile, "13800138000");
}
