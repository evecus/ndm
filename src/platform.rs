//! 平台相关能力：系统代理、开机自启、文件定位、系统托盘。
//! Windows 使用注册表 / 原生托盘；其它平台提供安全降级实现，便于开发与测试。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayAction {
    None,
    Show,
    Quit,
}

/// 解析系统代理设置，返回可直接交给 reqwest 的代理 URL。
#[cfg(not(windows))]
pub fn system_proxy_for(scheme: &str) -> Option<String> {
    let upper = if scheme.eq_ignore_ascii_case("https") {
        "HTTPS_PROXY"
    } else {
        "HTTP_PROXY"
    };
    for key in [upper, "ALL_PROXY", &upper.to_ascii_lowercase(), "all_proxy"] {
        if let Ok(v) = std::env::var(key) {
            let v = v.trim().to_string();
            if !v.is_empty() {
                return Some(v);
            }
        }
    }
    None
}

#[cfg(windows)]
pub fn system_proxy_for(scheme: &str) -> Option<String> {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};
    use winreg::RegKey;

    let key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(
            r"Software\Microsoft\Windows\CurrentVersion\Internet Settings",
            KEY_READ,
        )
        .ok()?;
    let enabled: u32 = key.get_value("ProxyEnable").unwrap_or(0);
    if enabled == 0 {
        return None;
    }
    let raw: String = key.get_value("ProxyServer").ok()?;
    let raw = raw.trim().to_string();
    if raw.is_empty() {
        return None;
    }

    let want = if scheme.eq_ignore_ascii_case("https") {
        "https"
    } else {
        "http"
    };
    let mut picked = None;
    for part in raw.split(';') {
        if let Some((k, v)) = part.split_once('=') {
            if k.trim().eq_ignore_ascii_case(want) {
                picked = Some(v.trim().to_string());
                break;
            }
        }
    }
    let host_port = picked.unwrap_or_else(|| {
        // "127.0.0.1:7890" 这种所有协议共用的简单格式
        raw.split(';').next().unwrap_or("").trim().to_string()
    });
    if host_port.is_empty() || host_port.contains('=') {
        return None;
    }
    if host_port.to_ascii_lowercase().starts_with("socks=") {
        return Some(format!("socks5://{}", host_port.trim_start_matches("socks=")));
    }
    Some(format!("http://{host_port}"))
}

/// 开机自启（Windows 写 HKCU Run 项）。
#[cfg(windows)]
pub fn set_auto_start(enable: bool) {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_WRITE};
    use winreg::RegKey;

    let result = (|| -> std::io::Result<()> {
        let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(
            r"Software\Microsoft\Windows\CurrentVersion\Run",
            KEY_WRITE,
        )?;
        if enable {
            let exe = std::env::current_exe()?;
            let cmd = format!("\"{}\" --minimized", exe.display());
            key.set_value("NDM-RS", &cmd)?;
        } else {
            let _ = key.delete_value("NDM-RS");
        }
        Ok(())
    })();
    if let Err(e) = result {
        log::warn!("设置开机自启失败: {e}");
    }
}

#[cfg(not(windows))]
pub fn set_auto_start(_enable: bool) {}

/// 在资源管理器中定位文件。
pub fn reveal_file(path: &std::path::Path) {
    #[cfg(windows)]
    {
        let p = path.to_string_lossy().replace('/', "\\");
        let _ = std::process::Command::new("explorer")
            .arg(format!("/select,{p}"))
            .spawn();
    }
    #[cfg(not(windows))]
    {
        let dir = path.parent().unwrap_or(std::path::Path::new("."));
        let _ = open::that(dir);
    }
}

/// 打开下载目录。
pub fn open_dir(path: &std::path::Path) {
    let _ = open::that(path);
}

#[cfg(not(windows))]
pub struct Tray;

#[cfg(not(windows))]
impl Tray {
    pub fn new() -> Self {
        Tray
    }
    pub fn poll(&self) -> TrayAction {
        TrayAction::None
    }
}

#[cfg(windows)]
pub struct Tray {
    _icon: Option<tray_icon::TrayIcon>,
    show_id: Option<tray_icon::menu::MenuId>,
    quit_id: Option<tray_icon::menu::MenuId>,
}

#[cfg(windows)]
impl Tray {
    pub fn new() -> Self {
        use tray_icon::{menu::Menu, menu::MenuItem, Icon, TrayIconBuilder};

        let icon = (|| -> Option<Icon> {
            let img = image::load_from_memory_with_format(
                include_bytes!("../assets/ndm.png"),
                image::ImageFormat::Png,
            )
            .ok()?
            .to_rgba8();
            let (w, h) = img.dimensions();
            Icon::from_rgba(img.into_raw(), w, h).ok()
        })();

        let mut tray = Tray {
            _icon: None,
            show_id: None,
            quit_id: None,
        };

        let menu = Menu::new();
        let show = MenuItem::new("显示主窗口", true, None);
        let quit = MenuItem::new("退出 NDM-RS", true, None);
        if menu.append_items(&[&show, &quit]).is_err() {
            return tray;
        }
        tray.show_id = Some(show.id().clone());
        tray.quit_id = Some(quit.id().clone());

        let mut builder = TrayIconBuilder::new();
        if let Some(i) = icon {
            builder = builder.with_icon(i);
        }
        builder = builder.with_tooltip("NDM-RS 下载管理器");
        match builder.with_menu(Box::new(menu)).build() {
            Ok(t) => tray._icon = Some(t),
            Err(e) => log::warn!("创建系统托盘失败: {e}"),
        }
        tray
    }

    pub fn poll(&self) -> TrayAction {
        use tray_icon::{menu::MenuEvent, MouseButton, MouseButtonState, TrayIconEvent};

        while let Ok(ev) = MenuEvent::receiver().try_recv() {
            if Some(&ev.id) == self.show_id.as_ref() {
                return TrayAction::Show;
            }
            if Some(&ev.id) == self.quit_id.as_ref() {
                return TrayAction::Quit;
            }
        }
        while let Ok(ev) = TrayIconEvent::receiver().try_recv() {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = ev
            {
                return TrayAction::Show;
            }
        }
        TrayAction::None
    }
}

impl Default for Tray {
    fn default() -> Self {
        Self::new()
    }
}
