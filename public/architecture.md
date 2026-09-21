# RustIM 客户端架构设计文档

## 项目概述

RustIM 是一套跨平台即时通讯客户端解决方案，堪为现代化架构设计中的典范，支持：

- 📱 移动端：iOS / Android / 鸿蒙
- 🖥️ 桌面端：Windows / macOS / Linux
- 🌐 Web端：现代浏览器

架构核心设计理念：

- 🧠 核心逻辑统一由 Rust 实现，封装通信协议、状态同步、加解密、缓存存储、数据库操作等关键逻辑。
- 📱 移动端采用 Flutter 进行 UI 构建，通过 FFI 与 Rust 通信。
- 🖥️ 桌面端采用 Tauri + Web 技术构建 UI，通过 JS Bridge 与 Rust 通信。
- 🌐 Web 端使用 Vue/React 前端，通过 wasm-bindgen 调用 Rust 核心逻辑（IndexedDB 存储）。
- 🧩 所有平台统一采用异步消息总线，确保消息驱动式 UI 更新。

## 技术栈

### 核心逻辑层（Rust）

- `tokio`：异步运行时
- `serde`：数据序列化
- `tokio-tungstenite`：WebSocket 通信
- `sqlx`：SQLite 存储 + schema migration
- `ring`：加密库
- `ffi-support`、`flutter_rust_bridge`：Flutter 对接
- `wasm-bindgen`：WebAssembly 接口
- `mpsc`：异步事件通道

### UI 层（多平台适配）

| 平台     | 框架           | 与 Rust 接口方式        |
|----------|----------------|--------------------------|
| iOS/Android/鸿蒙 | Flutter        | FFI + `.so`/`.a`          |
| Windows/macOS/Linux | Tauri + Vue/React | JS Bridge              |
| Web 浏览器 | Vue/React      | WASM + IndexedDB         |

## 项目结构（推荐）

```
rust-im/
├── im-core/             # Rust 核心逻辑库
│   ├── src/
│   │   ├── lib.rs
│   │   ├── api/         # FFI/WASM 接口导出
│   │   ├── transport/   # WebSocket 连接与协议收发
│   │   ├── crypto/      # 加解密模块
│   │   ├── message/     # 消息结构、发送状态、撤回等
│   │   ├── session/     # 登录态、多账号管理
│   │   ├── storage/     # 本地存储：SQLite / IndexedDB
│   │   ├── push/        # 推送 token 映射管理
│   │   ├── sync/        # 多设备同步逻辑
│   │   └── bus/         # 异步消息事件总线
│   ├── migrations/      # sqlx schema migration
│   └── Cargo.toml
│
├── im-mobile/           # Flutter 移动端 UI
│   └── lib/
│       ├── rust_bridge/
│       ├── pages/
│       ├── services/
│       └── models/
│
├── im-pc/               # 桌面客户端 (Tauri + Web UI)
│   ├── src-tauri/
│   └── src/
│
├── im-web/              # 浏览器 SPA + Rust WASM
│   ├── src/
│   └── wasm/
│
├── shared/              # 公共资源
│   ├── assets/
│   └── proto/
│
├── ci/                  # 自动化构建脚本
│   ├── build_ios.sh
│   ├── build_android.sh
│   ├── build_pc.sh
│   ├── build_web.sh
│   └── github-actions.yml
└── README.md
```

## 本地消息存储设计（统一 Rust 实现）

| 平台 | 存储后端 | 接口方式 |
|--------|-------------|-------------|
| 移动端 | SQLite via FFI | Flutter 调用 Rust `.so` |
| 桌面端 | SQLite via sqlx | Rust 原生 |
| Web端 | IndexedDB via WASM | wasm-bindgen + idb |

统一接口定义：

```rust
pub trait MessageStorage {
    fn insert_message(&self, msg: Message) -> Result<()>;
    fn fetch_history(&self, session_id: &str, limit: usize) -> Vec<Message>;
}
```

## 状态同步机制

```rust
struct Message {
    id: String,
    session_id: String,
    device_id: String,
    sync_status: SyncStatus, // Synced, LocalOnly, Conflict
    ...
}
```

支持：
- 多端消息状态同步
- 游标拉取历史消息
- 已读、撤回广播

## 核心异步消息通道

- 使用 tokio::mpsc 构建异步消息总线
```rust
pub enum AppEvent {
    MessageReceived(Message),
    ConnectionChanged(ConnectionState),
    PushTokenUpdated(String),
}

pub struct MessageBus {
    sender: mpsc::Sender<AppEvent>,
    receiver: mpsc::Receiver<AppEvent>,
}
```

- 所有 UI 层监听此通道获取事件（消息/网络/登录等），不用关心底层实现
- 支持队列缓存、节流、合并等，避免频繁触发 UI 更新
- 消息解耦，便于多端适配

## 推送管理结构

```rust
// Rust 不处理推送注册，只处理 Token 映射和发送逻辑
pub struct PushToken {
    device_id: String,
    platform: PushPlatform, // iOS, Android, Harmony...
    token: String,
}
```

- 由平台端 Flutter 注册并通过 FFI 传入
- Rust 仅维护映射，不主动触发推送

## 数据库管理建议

- 使用 `sqlx::migrate!` 管理 SQLite 表结构版本
- Web IndexedDB 通过版本控制 + onupgradeneeded 迁移
- 所有表设计预留 `account_id`, `device_id`

## 安全设计

- TLS + WebSocket 双重加密
- 本地消息加密存储（可选）
- Rust 端统一加解密流程，避免 UI 层涉密
- 支持私钥本地加密保存（PIN解锁）

## 模块化设计建议

所有模块应按功能细分，建议使用以下模块布局：

```
im-core/
├── transport/   # WebSocket
├── message/     # 消息状态管理
├── storage/     # SQLite / IndexedDB
├── crypto/      # 加密
├── sync/        # 多设备同步
├── push/        # 推送 token
├── session/     # 账号登录/切换
└── bus/         # 事件消息流
```

## 可选拓展功能（后期）

- DevTools：调试消息流、状态
- Metrics：运行时采样上报
- ChatBot 插件：群聊自动应答模块
- Markdown 渲染、引用、回复消息支持

---
该文档为 RustIM 跨平台即时通讯项目的核心设计文档，后期甚至可以用它开发 CLI 聊天工具、Bot 接口、桌面端插件等。