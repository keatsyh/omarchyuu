//! Shared virtual-key names and egui key conversion for shortcut editors.
pub(crate) fn key_code(key: egui::Key) -> Option<u16> {
    let name = key.name();
    if name.len() == 1 {
        let b = name.as_bytes()[0].to_ascii_uppercase();
        if b.is_ascii_alphanumeric() {
            return Some(u16::from(b));
        }
    }
    if let Some(f) = name.strip_prefix('F').and_then(|n| n.parse::<u16>().ok())
        && (1..=24).contains(&f)
    {
        return Some(111 + f);
    }
    Some(match key {
        egui::Key::ArrowLeft => 37,
        egui::Key::ArrowUp => 38,
        egui::Key::ArrowRight => 39,
        egui::Key::ArrowDown => 40,
        egui::Key::Escape => 27,
        egui::Key::Tab => 9,
        egui::Key::Backspace => 8,
        egui::Key::Enter => 13,
        egui::Key::Space => 32,
        egui::Key::Insert => 45,
        egui::Key::Delete => 46,
        egui::Key::Home => 36,
        egui::Key::End => 35,
        egui::Key::PageUp => 33,
        egui::Key::PageDown => 34,
        _ => return None,
    })
}
pub(crate) fn label(key: u16, modifiers: u8) -> String {
    let mut words = Vec::new();
    for (bit, name) in [(1, "Ctrl"), (2, "Shift"), (4, "Alt"), (8, "Win")] {
        if modifiers & bit != 0 {
            words.push(name.to_owned());
        }
    }
    let key = match key {
        1 => "鼠标左键".into(),
        2 => "鼠标右键".into(),
        4 => "鼠标中键".into(),
        5 => "鼠标侧键1".into(),
        6 => "鼠标侧键2".into(),
        65..=90 | 48..=57 => char::from_u32(u32::from(key)).unwrap_or('?').to_string(),
        112..=135 => format!("F{}", key - 111),
        8 => "Backspace".into(),
        9 => "Tab".into(),
        13 => "Enter".into(),
        27 => "Esc".into(),
        32 => "Space".into(),
        37 => "←".into(),
        38 => "↑".into(),
        39 => "→".into(),
        40 => "↓".into(),
        _ => format!("Key {}", key),
    };
    words.push(key);
    words.join(" + ")
}
