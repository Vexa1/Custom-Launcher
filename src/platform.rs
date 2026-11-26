//! Small platform-naming helpers so the UI reads correctly on both systems.

pub fn os_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "macOS"
    } else {
        "Windows"
    }
}

pub fn tray_label() -> &'static str {
    if cfg!(target_os = "macos") {
        "menu bar"
    } else {
        "system tray"
    }
}

pub fn tray_label_titlecase() -> &'static str {
    if cfg!(target_os = "macos") {
        "Menu bar"
    } else {
        "System tray"
    }
}

pub fn tray_card_title() -> &'static str {
    if cfg!(target_os = "macos") {
        "Menu Bar & Startup"
    } else {
        "System Tray & Startup"
    }
}

pub fn tray_help_text() -> &'static str {
    if cfg!(target_os = "macos") {
        "Click the menu bar icon to reopen the launcher. Right-click it for Settings, Discord RPC, updates, or Quit."
    } else {
        "Right-click the tray icon to open the launcher, toggle Discord RPC, check updates, or quit."
    }
}

pub fn player_label() -> &'static str {
    if cfg!(target_os = "macos") {
        "The Roblox player"
    } else {
        "RobloxPlayerBeta.exe"
    }
}
