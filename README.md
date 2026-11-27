# NDM-RS（Rust + GPUI 版）

用 Rust 编写的 **Windows 桌面下载管理器**，可直接搭配官方 **NeatDownloadManager Extension 1.9.92** 使用。

这一版只把界面从 **egui 0.31** 换成了 **GPUI**（Zed 编辑器的 GPU 加速 UI 框架）+ **gpui-component**。下载引擎、扩展协议、HLS、限速、代理、持久化格式都**没有改动**，`tests/integration.rs` 照样运行。

---

## 一、改了什么、没改什么

| 模块 | 状态 | 说明 |
| --- | --- | --- |
| `engine.rs` / `extension.rs` / `hls.rs` / `throttle.rs` | 未改动 | 下载、WebSocket 扩展桥、m3u8、令牌桶 |
| `model.rs` / `settings.rs` / `util.rs` | 未改动 | `settings.json` / `tasks.json` 格式不变，可直接沿用旧数据 |
| `platform.rs` | 未改动 | 托盘（tray-icon）、系统代理、开机自启、打开目录 |
| `main.rs` / `build.rs` | 未改动 | 命令行参数、`--headless`、图标资源 |
| `app.rs` | **重写** | GPUI 应用、窗口生命周期、托盘轮询、刷新调度、对话框逻辑 |
| `theme.rs` | **重写** | 色板改为 `gpui::Rgba`，渐变改用原生 `linear_gradient` |
| `ui/*` | **新增** | `chrome`（工具栏/侧栏/状态栏/通知）、`tasks`（任务卡片）、`dialogs`（新建/设置/关于）、`widgets`（组件） |

依赖变化：去掉 `eframe` / `egui`，加入 `gpui 0.2.2` 与 `gpui-component 0.4`；`image` 只在 Windows 下用于托盘图标。

---

## 二、架构变化

**1. 刷新方式**
egui 版每帧都读一遍 `AppState`，为驱动动画在有任务时 33ms 重绘一次整窗。GPUI 版用后台定时器调用 `notify`：有活动任务时 200ms 一次，空闲时 500ms 一次（刷新速度、扩展连接状态、通知过期）。

**2. 动画**
流光进度条、呼吸状态灯、空状态的脉冲圆环都改为 GPUI 的 `with_animation` 声明式动画，由框架按显示器刷新率绘制。原来用 `Painter` 逐像素画竖线拼渐变，现在直接用 `linear_gradient` 背景。

**3. 关闭到托盘**
egui 版拦截关闭事件后把窗口设为不可见。GPUI 版换成更简单的做法：
- 开启「关闭窗口时最小化到系统托盘」时，关闭窗口但进程继续运行，下载不受影响；
- 托盘「显示主窗口」时，如果窗口还在就激活它，不在就重新创建（状态都在 `Arc<AppState>` 里，不会丢）；
- 托盘「退出」或关闭了托盘选项时，先 `save_all()` 再退出。
- `--minimized` 启动时不创建窗口，只留托盘。非 Windows 平台没有托盘，会照常打开窗口。

**4. 输入控件**
- 文本框用 gpui-component 的 `Input`（自带 IME、选区、剪贴板、撤销），代理密码框开启掩码。
- 连接数、同时任务数改为 `− 值 +` 步进器；全局限速是文本框，保存时校验必须为整数。
- 勾选项、代理模式（分段按钮）用组件自己绘制。

**5. 中文字体**
egui 必须手动读取 `C:\Windows\Fonts` 里的 TrueType 字体才能显示中文，找不到时会出现方块和警告。GPUI 使用系统文本栈（Windows 上是 DirectWrite），中文和 ▶ ♪ ⚙ 这类符号会自动回退到系统字体。所以字体探测、`NDM_FONT` 环境变量和缺字警告都删掉了。界面字体是 Windows 上的 `Microsoft YaHei UI`、macOS 上的 `PingFang SC`、Linux 上的 `Noto Sans CJK SC`。

**6. 快捷键**
`Ctrl+N` 新建任务，`Esc` 关闭对话框。

---

## 三、编译

```powershell
# Windows（MSVC + Windows 10 SDK）
rustup default stable-x86_64-pc-windows-msvc
cargo build --release
# 或
.\scripts\build-windows.ps1
```

Linux 上运行 `cargo test` 需要先装 GPUI 的依赖：

```bash
sudo apt-get install -y libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev \
  libx11-xcb-dev libxcb1-dev libvulkan-dev libfontconfig-dev libfreetype-dev pkg-config clang
cargo test --all-targets
```

> 需要 Rust 1.85 或更高版本（GPUI 要求）。旧的 `Cargo.lock` 是按 egui 生成的，已删除；第一次编译通过后请提交新的 lock 文件，再在 CI 里加回 `--locked`。

---

## 四、运行

参数和以前一样：

```powershell
.\ndm-rs.exe                      # 图形界面
.\ndm-rs.exe --minimized          # 只启动托盘
.\ndm-rs.exe --headless           # 无界面常驻
.\ndm-rs.exe --add-url "https://example.com/a.zip" --dir D:\Downloads --threads 16
```

官方扩展固定连接 `ws://127.0.0.1:10007/download`，请使用默认端口。扩展协议细节与旧版 README 相同（握手回显 `neatextension.v1`、先发 `nowaiting` / `Version 2.0` / `ShowPanelChrome=1`，字段 `1:`~`9:`、`Content-Type:`、`Content-Disposition:`、`__0NeatPostData9__:` 等）。

---

## 五、注意：GPUI 生态版本变化快

GPUI 还没到 1.0，gpui-component 几乎每周发版，经常有破坏性改动。本项目按 `gpui 0.2.2` + `gpui-component 0.4` 的 API 编写，**还没有实际编译过**。如果报错，最可能是这些地方：

| 位置 | 可能的变化 |
| --- | --- |
| `gpui_component::input::Input` | 旧版叫 `TextInput` |
| `InputState::masked(true)` | 掩码方法名可能不同 |
| `Root::new(view, window, cx)` | 参数在 `AnyView` / `impl Into<AnyView>` 之间变过 |
| `Theme::change(ThemeMode::Dark, None, cx)` | 第二个参数类型可能变化 |
| `cx.spawn(async move \|this, cx\| ...)` | 旧版 GPUI 的 spawn 闭包签名不同 |
| `("name", usize)` 形式的元素 ID | 新版也接受 `u64` |

另外 tray-icon 在 Windows 上要求创建它的线程有消息循环。GPUI 主线程本身就在跑 Win32 消息循环，所以托盘是在 `Application::run` 里创建的；如果托盘菜单没有反应，先检查这一点。

---

## 许可

MIT
