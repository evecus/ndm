//! UI 层：每个文件通过 `impl NdmApp` 返回一块 GPUI 元素树。
//!
//! - `chrome`  ：顶部工具栏、左侧分类栏、底部状态栏、右下角通知
//! - `tasks`   ：任务卡片列表与空状态
//! - `dialogs` ：新建任务 / 设置 / 关于
//! - `widgets` ：图标块、状态胶囊、流光进度条、呼吸灯、按钮等复用组件

pub mod chrome;
pub mod dialogs;
pub mod tasks;
pub mod widgets;
