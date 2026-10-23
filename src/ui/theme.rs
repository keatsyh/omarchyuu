//! Shared visual language for every native UI surface.
//! Omarchy workspace: neutral canvas, terminal tabs, square controls and quiet surfaces.
//! Layout is page-specific; palette, typography and control density live here.
use egui::{Color32, FontId, Stroke};

pub const BG: Color32 = Color32::from_rgb(24, 28, 29);
pub const SIDEBAR: Color32 = Color32::from_rgb(19, 23, 24);
pub const SURFACE: Color32 = Color32::from_rgb(32, 38, 39);
pub const LINE: Color32 = Color32::from_rgb(67, 79, 80);
pub const TEXT: Color32 = Color32::from_rgb(224, 232, 229);
pub const MUTED: Color32 = Color32::from_rgb(158, 177, 171);
pub const ACCENT: Color32 = Color32::from_rgb(142, 196, 190);
pub const GREEN: Color32 = Color32::from_rgb(166, 196, 140);
pub const AMBER: Color32 = Color32::from_rgb(224, 186, 126);
pub const RED: Color32 = Color32::from_rgb(230, 141, 141);
pub const DANGER_FILL: Color32 = Color32::from_rgb(161, 56, 67);
pub const CONTROLLED_TITLE_BG: Color32 = Color32::from_rgb(85, 34, 43);
pub const CONTROLLED_TITLE_LINE: Color32 = Color32::from_rgb(140, 59, 72);
pub const CONTROLLED_TITLE_TEXT: Color32 = Color32::from_rgb(255, 235, 238);
pub const CONTROLLED_TITLE_BRAND_MAX: f32 = 210.0;
pub const CONTROLLED_TITLE_GAP: f32 = 12.0;
pub const HOVER: Color32 = Color32::from_rgb(40, 49, 50);
pub const WINDOW_BORDER: Color32 = Color32::from_rgb(85, 108, 108);
pub const BORDER_FOCUS: Color32 = Color32::from_rgb(142, 196, 191);
pub const SELECTED: Color32 = Color32::from_rgb(42, 60, 59);
pub const SELECTION: Color32 = Color32::from_rgb(48, 68, 66);
pub const ACTIVE: Color32 = Color32::from_rgb(56, 78, 75);
pub const DISABLED: Color32 = Color32::from_gray(90);
pub const VIEWER_SCRIM: Color32 = Color32::from_rgba_premultiplied(8, 10, 14, 232);
pub const CONNECTION_WALLPAPER_DIM: u8 = 166;
pub const CONNECTION_WALLPAPER_DETAIL_DIM: u8 = 212;

/// Resolve semantic UI tokens against Omarchy's active palette once per process.
/// Protocol colors, textures, QR codes and annotation ink are deliberately untouched.
pub fn color(token: Color32) -> Color32 {
    static PALETTE: std::sync::LazyLock<Palette> = std::sync::LazyLock::new(Palette::load);
    PALETTE.resolve(token)
}

/// Pick a readable label on an arbitrary Omarchy accent, including light themes.
pub fn on_accent() -> Color32 {
    let accent = color(ACCENT);
    let linear = |value: u8| {
        let value = value as f32 / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    let luminance =
        0.2126 * linear(accent.r()) + 0.7152 * linear(accent.g()) + 0.0722 * linear(accent.b());
    if luminance > 0.179 {
        Color32::BLACK
    } else {
        Color32::WHITE
    }
}

struct Palette {
    colors: std::collections::HashMap<String, Color32>,
}
impl Palette {
    fn load() -> Self {
        let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
        let state = std::env::var_os("XDG_STATE_HOME")
            .map(std::path::PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| home.as_ref().map(|p| p.join(".local/state")));
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(std::path::PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| home.as_ref().map(|p| p.join(".config")));
        let source = [state, config].into_iter().flatten().find_map(|base| {
            std::fs::read_to_string(base.join("omarchy/current/theme/colors.toml")).ok()
        });
        Self::parse(source.as_deref().unwrap_or_default())
    }
    fn parse(source: &str) -> Self {
        let colors = source
            .lines()
            .filter_map(|line| {
                let (key, value) = line.split_once('=')?;
                let value = value.trim().strip_prefix('"')?.split_once('"')?.0;
                let hex = value.strip_prefix('#')?;
                if hex.len() != 6 || !hex.is_ascii() {
                    return None;
                }
                let rgb = u32::from_str_radix(hex, 16).ok()?;
                Some((
                    key.trim().to_owned(),
                    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8),
                ))
            })
            .collect();
        Self { colors }
    }
    fn get(&self, key: &str, fallback: Color32) -> Color32 {
        self.colors.get(key).copied().unwrap_or(fallback)
    }
    fn resolve(&self, token: Color32) -> Color32 {
        if self.colors.is_empty() {
            return token;
        }
        match token {
            BG => self.get("background", BG),
            SIDEBAR => self.get(
                "dark_background",
                mix(
                    self.get("background", BG),
                    self.get("foreground", TEXT),
                    0.02,
                ),
            ),
            SURFACE => self.get(
                "lighter_background",
                mix(
                    self.get("background", BG),
                    self.get("foreground", TEXT),
                    0.055,
                ),
            ),
            TEXT => self.get("foreground", TEXT),
            MUTED => mix(
                self.get("background", BG),
                self.get("foreground", TEXT),
                0.74,
            ),
            LINE | WINDOW_BORDER => self.get("muted", LINE),
            ACCENT | BORDER_FOCUS => self.get("accent", ACCENT),
            GREEN => self.get("green", GREEN),
            AMBER => self.get("yellow", AMBER),
            RED => self.get("red", RED),
            SELECTED | SELECTION | ACTIVE => {
                mix(self.get("background", BG), self.get("accent", ACCENT), 0.22)
            }
            HOVER => mix(
                self.get("background", BG),
                self.get("foreground", TEXT),
                0.1,
            ),
            _ => token,
        }
    }
}

fn mix(background: Color32, foreground: Color32, amount: f32) -> Color32 {
    let channel = |bg: u8, fg: u8| (bg as f32 + (fg as f32 - bg as f32) * amount).round() as u8;
    Color32::from_rgb(
        channel(background.r(), foreground.r()),
        channel(background.g(), foreground.g()),
        channel(background.b(), foreground.b()),
    )
}

pub const BODY: f32 = 14.0;
// The file send target window exists on Windows only.
#[cfg_attr(not(windows), allow(dead_code))]
pub const FILE_SEND_WIDTH: f32 = 380.0;
#[cfg_attr(not(windows), allow(dead_code))]
pub const FILE_SEND_HEIGHT: f32 = 78.0;
#[cfg_attr(not(windows), allow(dead_code))]
pub const FILE_SEND_ROW_HEIGHT: f32 = 26.0;
#[cfg_attr(not(windows), allow(dead_code))]
pub const FILE_SEND_VISIBLE_ROWS: usize = 5;
pub const COMPACT_TEXT: f32 = 13.0;
pub const DIAGNOSTIC_POPUP_WIDTH: f32 = 320.0;
pub const TASK_PROGRESS_HEIGHT: f32 = 3.0;
pub const SMALL: f32 = 12.0;
pub const TINY: f32 = 11.0;
pub const ICON_STROKE: f32 = 1.35;
pub const VIEWER_CAPTION_BUTTON: f32 = 30.0;
/// 标题栏右侧动作区的固定宽度：每个按钮 VIEWER_CAPTION_BUTTON 再加 4px 间距，
/// 两头各留 4px。这是抢标题和屏标签地方换来的，加减按钮时要跟着改，
/// 不然排在最左边（最后添加）的按钮会被裁掉。
pub const VIEWER_ACTIONS_WIDTH: f32 = 248.0;
pub const VIEWER_FLOATING_WIDTH: f32 = 1100.0;
pub const VIEWER_FULLSCREEN_HANDLE_HIT_SIZE: egui::Vec2 = egui::vec2(340.0, 14.0);
pub const VIEWER_FULLSCREEN_HANDLE_SIZE: egui::Vec2 = egui::vec2(280.0, 4.0);
pub const VIEWER_FULLSCREEN_HANDLE_CENTER_Y: f32 = 10.0;
pub const SESSION_STATUS_WIDTH: f32 = 280.0;
pub const INPUT_WARNING_WIDTH: f32 = 360.0;
pub const INPUT_WARNING_MARGIN: f32 = 12.0;
pub const NOTIFICATION_WIDTH: f32 = 380.0;
pub const NOTIFICATION_MARGIN: f32 = 16.0;
pub const NOTIFICATION_PADDING: egui::Margin = egui::Margin::symmetric(12, 10);
pub const NOTIFICATION_GAP: f32 = 4.0;
pub const NOTIFICATION_HEADER_HEIGHT: f32 = 20.0;
pub const NOTIFICATION_ACTION_HEIGHT: f32 = 26.0;
pub const NOTIFICATION_ACTION_GAP: f32 = 8.0;
pub const NOTIFICATION_PROGRESS_HEIGHT: f32 = 4.0;
pub const NOTIFICATION_MIN_HEIGHT: f32 = 80.0;
pub const NOTIFICATION_MAX_HEIGHT: f32 = 520.0;
pub const MICRO: f32 = 10.0;
pub const SECTION: f32 = 16.0;
pub const DIALOG_TITLE: f32 = 21.0;
pub const UPDATE_DIALOG_WIDTH: f32 = 520.0;
pub const MAINTENANCE_WINDOW_SIZE: [f32; 2] = [580.0, 340.0];
pub const TAKEOVER_DIALOG_WIDTH: f32 = 440.0;
pub const DEVICE_STATUS_SIZE: [f32; 2] = [106.0, 26.0];
pub const REMOTE_UPGRADE_WIDTH: f32 = 408.0;
pub const DIALOG_MARGIN: i8 = 20;
pub const MESSAGE_DIALOG_WIDTH: f32 = 440.0;
pub const DIALOG_HEADER_HEIGHT: f32 = 32.0;
pub const DIALOG_HEADING: f32 = 16.0;
pub const DIALOG_HEADER_GAP: f32 = 16.0;
pub const DIALOG_ACTION_GAP: f32 = 20.0;
pub const DIALOG_ACTION_WIDTH: f32 = 104.0;
pub const UPDATE_MESSAGE_HEIGHT: f32 = 132.0;
pub const RELEASE_NOTES_HEIGHT: f32 = 300.0;
pub const TITLE: f32 = 25.0;
pub const BRAND: f32 = 17.0;
pub const CONTROL_HEIGHT: f32 = 34.0;
pub const INPUT_MARGIN: egui::Margin = egui::Margin::symmetric(12, 6);
pub const INPUT_COMPACT_MARGIN: egui::Margin = egui::Margin::symmetric(8, 4);
pub const MAPPING_WINDOW_SIZE: [f32; 2] = [920.0, 520.0];
pub const FILES_WINDOW_SIZE: [f32; 2] = [1280.0, 820.0];
pub const FILES_WINDOW_MIN: [f32; 2] = [940.0, 640.0];
pub const FILES_MARGIN: i8 = 12;
pub const FILES_TASK_HEIGHT: f32 = 210.0;
pub const FILES_ROW_HEIGHT: f32 = 30.0;
pub const FILES_DIALOG_WIDTH: f32 = 480.0;
pub const FILES_PANE_MIN: f32 = 420.0;
pub const FILES_PANE_MIN_HEIGHT: f32 = 300.0;
pub const FILES_SPLITTER: f32 = 8.0;
pub const FILES_QUEUE_ROW: f32 = 50.0;
pub const FILES_DATE_WIDTH: f32 = 118.0;
pub const FILES_SIZE_WIDTH: f32 = 78.0;
pub const FILES_NAV_HEIGHT: f32 = 30.0;
pub const FILES_NAV_GAP: f32 = 4.0;
pub const FILES_PATH_INSET: f32 = 4.0;
pub const FILES_BROWSER_HEADER: f32 = 144.0;
pub const MAPPING_WINDOW_MIN: [f32; 2] = [880.0, 480.0];
pub const MAPPING_MARGIN: i8 = 20;
pub const MAPPING_ROW_HEIGHT: f32 = 72.0;
pub const MAPPING_LINE_HEIGHT: f32 = 20.0;
pub const MAPPING_TABLE_HEADER: f32 = 34.0;
pub const MAPPING_DIALOG_WIDTH: f32 = 560.0;
pub const MAPPING_EMPTY_HEIGHT: f32 = 202.0;
pub const MAPPING_ENDPOINT_HEIGHT: f32 = 192.0;
pub const SWITCH_SIZE: egui::Vec2 = egui::vec2(60.0, 28.0);
pub const COMPACT_SWITCH_SIZE: egui::Vec2 = egui::vec2(46.0, 22.0);
pub const COMPACT_HEIGHT: f32 = 26.0;
pub const DIAGNOSTICS_BODY_MIN_HEIGHT: f32 = 350.0;
pub const DIAGNOSTICS_LABEL_WIDTH: f32 = 106.0;
pub const DIAGNOSTICS_ROW_HEIGHT: f32 = 28.0;
pub const DIAGNOSTICS_GAP: f32 = 10.0;
pub const DIAGNOSTICS_ACTION_WIDTH: f32 = 112.0;
pub const MENU_HEIGHT: f32 = 28.0;
pub const MENU_GROUP_GAP: f32 = 8.0;
pub const AUDIO_MENU_WIDTH: f32 = 320.0;
pub const AUDIO_WINDOW_SIZE: [f32; 2] = [480.0, 464.0];
pub const AUDIO_COMPACT_SIZE: [f32; 2] = [420.0, 140.0];
pub const SHORTCUT_ROW_HEIGHT: f32 = 56.0;
pub const SHORTCUT_LABEL_WIDTH: f32 = 184.0;
pub const PERFORMANCE_WIDTH: f32 = 540.0;
pub const PERFORMANCE_LABEL_WIDTH: f32 = 102.0;
pub const PERFORMANCE_ROW_HEIGHT: f32 = 44.0;
pub const PERFORMANCE_ALERT_SIZE: f32 = 48.0;
pub const PERFORMANCE_ALERT_GAP: f32 = 16.0;
pub const PERFORMANCE_ALERT_MARGIN: f32 = 20.0;
pub const SCREEN_TAB_HEIGHT: f32 = 30.0;
pub const WINDOW_TITLE_CONTENT_HEIGHT: f32 = 36.0;
// The controls add 4 px at the right; together with the frame stroke this
// matches the 7 px above/below a 30 px button in the 36 px caption content.
pub const WINDOW_TITLE_MARGIN: egui::Margin = egui::Margin {
    left: 10,
    right: 2,
    top: 3,
    bottom: 3,
};
pub const WINDOW_TITLE_STROKE: f32 = 1.0;
pub const WINDOW_CONTROLS_WIDTH: f32 = 106.0;
pub const CONNECTION_CONTENT_WIDTH: f32 = 440.0;
pub const CONNECTION_STAGE_WIDTH: f32 = 224.0;
pub const CONNECTION_DETAIL_TIME_WIDTH: f32 = 64.0;
pub const CONNECTION_DETAIL_TITLE_WIDTH: f32 = 160.0;
pub const VIEWER_IDENTITY_MAX_WIDTH: f32 = 200.0;
pub const WINDOW_LOGO_SIZE: f32 = 24.0;
pub const VIEWER_IDENTITY_GAP: f32 = 16.0;
pub const DEVICE_MENU_WIDTH: f32 = 248.0;
pub const DEVICE_MENU_ROW_HEIGHT: f32 = 40.0;
pub const DEVICE_MENU_ROW_GAP: f32 = 2.0;
pub const SCREEN_TAB_MIN_WIDTH: f32 = 108.0;
pub const SCREEN_TAB_MAX_WIDTH: f32 = 184.0;
pub const CONTEXT_MENU_WIDTH: f32 = 248.0;
pub const NAV_HEIGHT: f32 = 40.0;
pub const SIDEBAR_WIDTH: f32 = 184.0;
pub const AUTH_QR_SIZE: f32 = 216.0;
pub const AUTH_COUNTRY_WIDTH: f32 = 64.0;
pub const AUTH_CODE_ACTION_WIDTH: f32 = 128.0;
pub const ASSIST_ID_WIDTH: f32 = 216.0;
pub const ASSIST_COLUMN_GAP: f32 = 24.0;
pub const ASSIST_INPUT_WIDTH: f32 = 232.0;
pub const ASSIST_SHARE_WIDTH: f32 = 112.0;
pub const ASSIST_RECENT_SIZE: egui::Vec2 = egui::vec2(216.0, 52.0);
pub const ASSIST_CARD_MARGIN: i8 = 20;
pub const ASSIST_HEADER_MARGIN: egui::Margin = egui::Margin::symmetric(20, 12);
pub const CONTROL_RADIUS: u8 = 0;
pub const PANEL_RADIUS: u8 = 0;

pub fn typography(style: &mut egui::Style, height: f32) {
    let size = if height < CONTROL_HEIGHT {
        COMPACT_TEXT
    } else {
        BODY
    };
    style
        .text_styles
        .insert(egui::TextStyle::Body, FontId::monospace(size));
    style
        .text_styles
        .insert(egui::TextStyle::Button, FontId::monospace(size));
    style
        .text_styles
        .insert(egui::TextStyle::Small, FontId::monospace(SMALL));
    style
        .text_styles
        .insert(egui::TextStyle::Heading, FontId::monospace(TITLE));
}

pub fn configure(ctx: &egui::Context) {
    ctx.set_theme(egui::ThemePreference::Dark);
    let mut style = (*ctx.style_of(egui::Theme::Dark)).clone();
    style.visuals = egui::Visuals::dark();
    style.visuals.panel_fill = color(BG);
    style.visuals.window_fill = color(SIDEBAR);
    style.visuals.window_stroke = Stroke::new(1.0, color(ACCENT));
    style.visuals.window_shadow = egui::epaint::Shadow::NONE;
    style.visuals.popup_shadow = egui::epaint::Shadow::NONE;
    style.visuals.menu_corner_radius = CONTROL_RADIUS.into();
    style.visuals.extreme_bg_color = color(SIDEBAR);
    style.visuals.override_text_color = Some(color(TEXT));
    style.visuals.weak_text_color = Some(color(MUTED));
    style.visuals.error_fg_color = color(RED);
    style.visuals.warn_fg_color = color(AMBER);
    style.visuals.selection.bg_fill = color(SELECTION);
    style.visuals.selection.stroke = Stroke::new(1.0, color(ACCENT));
    style.visuals.window_corner_radius = PANEL_RADIUS.into();
    super::controls::configure(&mut style, CONTROL_HEIGHT);
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.interaction.selectable_labels = false;
    ctx.set_style_of(egui::Theme::Dark, style);
}

/// Graph colors communicate different node/port meanings within the same theme.
pub mod graph {
    use egui::Color32;
    pub const SOURCE: Color32 = Color32::from_rgb(77, 190, 178);
    pub const VIDEO: Color32 = Color32::from_rgb(101, 161, 240);
    pub const OVERLAY: Color32 = Color32::from_rgb(183, 136, 235);
    pub const INPUT: Color32 = Color32::from_rgb(232, 128, 137);
    pub const ANALYSIS: Color32 = Color32::from_rgb(225, 180, 91);
    pub const PORT_FRAME: Color32 = Color32::from_rgb(90, 160, 255);
    pub const PORT_DRAW: Color32 = Color32::from_rgb(100, 215, 150);
    pub const PORT_LAYER: Color32 = Color32::from_rgb(205, 145, 255);
    pub const PORT_DETECTIONS: Color32 = Color32::from_rgb(240, 185, 80);
    pub const PORT_INPUT: Color32 = Color32::from_rgb(240, 110, 110);
    pub const PORT_ACTIVATION: Color32 = Color32::from_rgb(240, 210, 110);
}
pub const ANNOTATION_BUTTON: f32 = 30.0;
pub const ANNOTATION_PANEL_MARGIN: i8 = 8;
pub const ANNOTATION_HEADER_HEIGHT: f32 = 22.0;
pub const ANNOTATION_ROW_GAP: f32 = 6.0;
pub const ANNOTATION_CLEAR_WIDTH: f32 = 360.0;
pub const ANNOTATION_WIDTH: f32 = 300.0;
pub const ANNOTATION_TOOL_SIZE: f32 = 32.0;
pub const ANNOTATION_TOOL_GAP: f32 = 4.0;
pub const ANNOTATION_PICKER_WIDTH: f32 = 256.0;
pub const ANNOTATION_COLOR_WIDTH: f32 = 100.0;
pub const ANNOTATION_SIZE_WIDTH: f32 = 96.0;
pub const ANNOTATION_BOARD_PICKER_WIDTH: f32 = 220.0;
pub const ANNOTATION_BOARD_SWATCH: egui::Vec2 = egui::vec2(68.0, 54.0);
pub const ANNOTATION_BRAND_TILE: [u8; 3] = [24, 28, 29];
pub const ANNOTATION_BRAND_PADDING: f32 = 24.0;
pub const ANNOTATION_BRAND_LOGO_SIZE: f32 = 56.0;
pub const ANNOTATION_BRAND_TITLE_SIZE: f32 = 26.0;
pub const ANNOTATION_BRAND_INFO_SIZE: f32 = 13.0;
pub const ANNOTATION_BRAND_GAP: f32 = 16.0;
pub const ANNOTATION_BRAND_BLUE: [u8; 3] = [142, 196, 190];
pub const ANNOTATION_BRAND_WHITE: [u8; 3] = [224, 232, 229];
pub const ANNOTATION_BRAND_ON_LIGHT: [[u8; 3]; 2] = [[45, 58, 76], [104, 119, 137]];
pub const ANNOTATION_BRAND_ON_DARK: [[u8; 3]; 2] = [[223, 231, 241], [153, 174, 191]];
pub const ANNOTATION_COLOR_PLANE_HEIGHT: f32 = 148.0;
pub const ANNOTATION_COLOR_BAR_HEIGHT: f32 = 14.0;
pub const ANNOTATION_BOARD_COLORS: [(&str, [u8; 3]); 3] = [
    ("白色", [255, 255, 255]),
    ("深色", [24, 28, 32]),
    ("绿色", [20, 74, 58]),
];
pub const ANNOTATION_COLORS: [[u8; 3]; 6] = [
    [255, 68, 68],
    [255, 190, 64],
    [83, 211, 159],
    [67, 145, 255],
    [198, 115, 255],
    [255, 255, 255],
];

pub const POINTER_DEFAULT_SIZE: f32 = 20.0;
pub const POINTER_LEFT_CLICK: Color32 = ACCENT;
pub const POINTER_RIGHT_CLICK: Color32 = AMBER;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bold_labels_and_primary_buttons_keep_readable_contrast() {
        fn luminance(c: Color32) -> f64 {
            let channel = |v: u8| {
                let v = v as f64 / 255.0;
                if v <= 0.04045 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b())
        }
        let contrast = |a: Color32, b: Color32| {
            let (a, b) = (luminance(a), luminance(b));
            (a.max(b) + 0.05) / (a.min(b) + 0.05)
        };
        let ctx = egui::Context::default();
        configure(&ctx);
        let style = ctx.style_of(egui::Theme::Dark);
        let strong = style.visuals.strong_text_color();
        assert!(
            contrast(strong, style.visuals.widgets.active.bg_fill) >= 4.5,
            "pressed button contrast"
        );
        for background in [BG, SIDEBAR, SURFACE] {
            assert!(
                contrast(strong, color(background)) >= 4.5,
                "bold label contrast on {background:?}"
            );
        }
        assert!(
            contrast(on_accent(), color(ACCENT)) >= 4.5,
            "primary label contrast"
        );
    }

    #[test]
    fn damaged_or_partial_omarchy_themes_keep_ui_usable() {
        let palette = Palette::parse(
            "background = \"bad\"\nforeground = \"#不是颜色\"\nred = \"#ff0000\" # alert\n",
        );
        assert_eq!(palette.resolve(BG), BG);
        assert_eq!(palette.resolve(TEXT), TEXT);
        assert_eq!(palette.resolve(RED), Color32::RED);
        assert_eq!(palette.resolve(Color32::WHITE), Color32::WHITE);
    }

    #[test]
    fn shell_and_controls_share_omarchy_colors_without_dimming_labels() {
        let palette = Palette::parse(
            "background = \"#f0f0f0\"\nforeground = \"#202020\"\nmuted = \"#eeeeee\"\naccent = \"#004488\"\n",
        );
        assert_eq!(palette.resolve(BG), Color32::from_rgb(240, 240, 240));
        assert_eq!(palette.resolve(MUTED), Color32::from_rgb(86, 86, 86));
        assert_eq!(palette.resolve(ACCENT), Color32::from_rgb(0, 68, 136));
        assert_eq!(Palette::parse("").resolve(BG), BG);
    }
}
