# NDM-RS

用 Rust 编写的 **Windows 桌面下载管理器**，可直接搭配官方 **NeatDownloadManager Extension 1.9.92** 使用，无需自己写浏览器插件。

官方扩展会把网页里捕获的下载链接（含 Cookie、Referer、文件名、HLS 地址等）通过本地 WebSocket 推给本程序，本程序负责高速下载、断点续传、任务管理与桌面界面。

---

## 功能

| 能力 | 说明 |
| --- | --- |
| 多连接分段下载 | 默认 8 段，先探测 `Accept-Ranges`/`Content-Range`，不支持时自动退化为单连接 |
| 断点续传 | 分片写入临时目录并持久化进度，重启后从已写字节继续 |
| 浏览器扩展桥 | `ws://127.0.0.1:10007/download`，子协议 `neatextension.v1`，完整兼容官方扩展 |
| HLS / m3u8 | master/media 播放列表、AES-128 解密、分片并发下载与顺序合并 |
| 请求还原 | 透传 `Referer`、`Cookie`、`Origin`、`User-Agent`、`Authorization` 等头，支持 POST 数据 |
| 调度与限速 | 同时任务数队列、全局令牌桶限速（KB/s）、暂停/继续/取消/失败重试 |
| 代理 | 直连 / 跟随 Windows 系统代理 / HTTP / SOCKS5，支持账号密码 |
| 桌面集成 | egui 中文界面、系统托盘、开机自启、打开文件所在目录、完成提示、分类目录 |
| 无界面模式 | `--headless` 仅跑引擎与扩展服务，适合后台常驻或服务器 |

---

## 直接运行（预编译版）

交付包内附 **`ndm-rs.exe`**（Windows x64）。它由本源码用 `x86_64-pc-windows-gnu` 交叉编译生成，已静态链接 C 运行时，**只依赖 Windows 系统自带 DLL**，无需安装 Rust 或任何运行库，图标资源也已嵌入。

- 双击 → 启动图形界面（Release 为 GUI 子系统，不会弹出黑色控制台窗口）
- 命令行 → `ndm-rs.exe --headless` / `--minimized` / `--add-url ...`（从 cmd / PowerShell 启动时仍继承其控制台，输出可见）
- 首次启动会在 `%APPDATA%\ndm-rs\` 生成 `settings.json` 与 `tasks.json`

如需修改代码、换用 MSVC 工具链或重新构建，见下一节。

---

## 编译（目标 Windows）

在 Windows 上安装 [rustup](https://rustup.rs)（**需要 Rust 1.81 或更高版本**，`rustup default stable` 即可满足）与 **Visual Studio Build Tools（MSVC + Windows 10 SDK）**，然后：

```powershell
rustup default stable-x86_64-pc-windows-msvc
cd ndm-rs
cargo build --release
```

产物为 `target\release\ndm-rs.exe`，单文件即可运行，无需额外 DLL（TLS 使用 rustls + ring，纯静态链入）。

Windows 图标与版本信息由 `build.rs` 通过 `winresource` 嵌入；若环境缺少 `rc.exe`，会自动跳过并给出 warning，不影响编译。

> Linux/macOS 下可用 `cargo check` 与 `cargo test` 验证逻辑；GUI 在 Linux 上编译需要 X11/wayland 开发库，托盘与开机自启等 Windows 特性由 `cfg(windows)` 隔离，非 Windows 平台自动降级为空实现。

也可以直接用提供的脚本：

```powershell
.\scripts\build-windows.ps1
```

---

## 运行

```powershell
.\ndm-rs.exe                      # 启动图形界面
.\ndm-rs.exe --minimized          # 启动后最小化到托盘
.\ndm-rs.exe --headless           # 无界面常驻
.\ndm-rs.exe --add-url "https://example.com/a.zip" --dir D:\Downloads --threads 16
```

| 参数 | 说明 |
| --- | --- |
| `--headless` | 无窗口运行，每 5 秒在控制台打印一次任务/速度/扩展连接状态 |
| `--minimized` | 启动后最小化到系统托盘 |
| `--port <端口>` | 扩展 WebSocket 监听端口，**默认 10007** |
| `--data-dir <目录>` | 配置与任务记录目录（也可用环境变量 `NDM_DATA_DIR`） |
| `--add-url <URL>` | 启动时添加下载任务，可重复多次 |
| `--dir <目录>` | `--add-url` 任务的保存目录 |
| `--threads <数量>` | `--add-url` 任务的连接数 |
| `-h` / `--help` | 帮助 |
| `-V` / `--version` | 版本号 |

> **重要**：官方扩展把 `ws://127.0.0.1:10007/download` 写死在 `bg.js` 里，没有端口设置项。要让官方扩展连上，必须使用默认端口 **10007**。`--port` 仅供 headless 测试或第三方客户端使用。

---

## 安装浏览器扩展

直接安装官方 **NeatDownloadManager Extension 1.9.92**（Chrome / Edge / 基于 Chromium 的浏览器均可），本仓库不包含也不需要任何扩展代码。

1. 打开 `chrome://extensions`，开启「开发者模式」；
2. 把官方 `.crx` 拖入页面，或解压后「加载已解压的扩展程序」；
3. 先启动 `ndm-rs.exe`，扩展会自动连接 `127.0.0.1:10007`；
4. 扩展图标显示已连接后，网页里的下载链接 / 视频即可被接管。

界面「设置 → 关于」里也会显示扩展连接状态；未连接时请检查 10007 端口是否被占用、防火墙是否放行回环地址。

---

## 扩展协议（实现要点）

**握手**：扩展发起 `GET /download`，带 `Upgrade: websocket`、`Sec-WebSocket-Protocol: neatextension.v1`。服务端校验 `Sec-WebSocket-Key` 并返回 `101 Switching Protocols`，**必须回显** `Sec-WebSocket-Protocol: neatextension.v1`，否则官方扩展会断开。

**握手后服务端先发三条控制消息**：

```
nowaiting            # 告诉扩展：由下载器自行探测文件大小，不用等待
Version 2.0          # 协议版本，扩展据此判断可用特性
ShowPanelChrome=1    # 启用网页媒体抓取面板
```

**扩展推送的下载请求**为一段文本，逐行 `键: 值`：

| 字段 | 含义 |
| --- | --- |
| `1:` | HTTP 方法 |
| `2:` | 下载 URL |
| `3:` | 文件名 |
| `4:` / `9:` | 标题 / 备注 |
| `5:` | 来源页（Referer 兜底） |
| `6:` | 类型 `normal` / `media` / `hls` |
| `7:` | 大小提示 |
| `8:` | 响应 Content-Type |
| `Content-Type:`（字段 10） | POST 请求体的 Content-Type |
| `Content-Disposition:`（字段 11） | 响应 Content-Disposition，用于提取文件名，不作为请求头发送 |
| `Referer:` `Cookie:` `Origin:` `X-*` | 需要透传的请求头 |
| `__0NeatPostData9__:` | POST 数据（固定为最后一段，可含换行） |

官方 `bg.js` 的 `V.I` 把字段 10 / 11 以 `Content-Type:` / `Content-Disposition:` **头行**形式发送（而非 `10:` / `11:` 数字行），解析器两种形式都兼容；`Content-Disposition` 只用于提取文件名，绝不回传给服务器。

同时兼容旧版/第三方扩展的 `fileName:` / `fEx:` / `fS:` / `fDu:` 字段。文件名优先级：显式字段 → Content-Disposition → URL 路径；无扩展名时用 `fEx` 或响应 Content-Type 补全。

---

## 源码结构

| 文件 | 职责 |
| --- | --- |
| `src/model.rs` | `AppState` / `Task` / `Settings` 状态模型、任务持久化、事件与速度统计 |
| `src/engine.rs` | HTTP 下载引擎：Range 探测、分段并发、断点续传、合并、请求头与代理构造、任务调度 |
| `src/extension.rs` | 手写 WebSocket 握手 + 官方协议解析与桥接服务 |
| `src/hls.rs` | m3u8 解析、AES-128-CBC 解密、分片并发抓取与合并 |
| `src/app.rs` | egui 桌面界面：任务列表、工具栏、分类、新建任务、设置、关于、中文字体加载 |
| `src/platform.rs` | Windows 托盘、系统代理读取、开机自启、打开目录（`cfg(windows)` 隔离） |
| `src/settings.rs` | 设置项与持久化 |
| `src/throttle.rs` | 全局令牌桶限速 |
| `src/util.rs` | 文件名、路径、时间、体积/速度格式化等工具 |
| `tests/integration.rs` | 本地 HTTP/Range 服务器上的端到端测试 |

数据默认存放在 `%APPDATA%\ndm-rs\`（`settings.json`、`tasks.json`、`parts/` 临时分片），可用 `NDM_DATA_DIR` 覆盖，便于便携版或多实例测试。

---

## 测试

```bash
cargo test --all-targets
```

集成测试会在本机起一个支持 Range 的临时 HTTP 服务，验证：分段下载结果与原文件字节一致、断点续传（中断后重启继续）、扩展 payload 解析、WebSocket 握手与握手后控制消息、限速与暂停/取消等。

---

## 已知限制

- 托盘、开机自启、系统代理读取仅在 Windows 生效；Linux/macOS 上自动降级。
- 中文界面依赖系统自带 TrueType 中文字体（Windows 下会自动尝试 `C:\Windows\Fonts` 中的 `simhei.ttf` / `Deng.ttf` / `msyh.ttc` 等）。若系统确实没有，界面会给出提示，可用环境变量 `NDM_FONT` 指定任意中文 `.ttf` 路径。
- 官方扩展的媒体嗅探面板只用 `ShowPanelChrome=1` 打开基础交互；未实现 MPEG-DASH、以及官方客户端的部分高级设置项。
- 动态分段目前保留设置项但未启用运行时重分配。

---

## 许可

MIT
