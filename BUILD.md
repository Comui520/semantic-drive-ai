# Build and Run Guide

本项目是 Tauri 2 桌面应用。真正的项目根目录是：

```text
D:\aiccfuture\semantic-drive-ai
```

不要在外层归档目录 `D:\aiccfuture` 执行项目命令。

## 环境要求

- Windows 10/11（当前优先验证平台）
- Node.js 22+
- Rust stable（项目最低 Rust 版本见 `src-tauri/Cargo.toml`）
- WebView2 Runtime
- Windows C++/MinGW 构建工具链

## 安装依赖

```powershell
cd D:\aiccfuture\semantic-drive-ai
npm install
```

验证 Rust 依赖：

```powershell
cd src-tauri
cargo check
cd ..
```

## 开发模式

推荐直接运行完整桌面应用：

```powershell
cd D:\aiccfuture\semantic-drive-ai
npm run tauri:dev
```

Tauri 会先执行 `npm run dev` 启动 Vite，再编译 Rust 后端并打开桌面窗口。

只运行前端：

```powershell
npm run dev
```

这只适合检查 React 页面；由于没有 Tauri IPC，扫描、文件操作和 API 设置等功能不能完整工作。

## 质量检查

```powershell
npm run lint
npm run build
cd src-tauri
cargo check
```

Rust 单元测试：

```powershell
cd src-tauri
cargo test
```

如果 Windows GNU 环境缺少匹配的 MinGW 运行时 DLL，测试二进制可能在启动阶段出现 `STATUS_ENTRYPOINT_NOT_FOUND`。这不是 Rust 编译错误；优先使用 `cargo check` 验证编译，或修复本机工具链运行时后再执行测试。

## 生产构建

```powershell
cd D:\aiccfuture\semantic-drive-ai
npm run tauri:build
```

安装包和 bundle 通常位于：

```text
src-tauri\target\release\bundle\
```

常见子目录包括：

```text
src-tauri\target\release\bundle\msi\
src-tauri\target\release\bundle\nsis\
```

`src-tauri\target\` 是编译缓存和构建产物，已被 Git 忽略，不要提交。

## 首次运行

1. 启动 `npm run tauri:dev`。
2. 在“设置”中选择工作区目录。
3. 填写 Chat / Agent API 的 Base URL、模型和 API Key。
4. 如需云端语义检索，再填写 Embedding API。
5. 在“智能搜索”中执行首次扫描。
6. 在“智能助手”中测试搜索、计划预览和确认执行。

## 工作区说明

应用只管理当前工作区及其子目录。首次运行时，如果用户没有选择工作区，程序优先使用系统 Documents 目录，避免误扫安装目录、源码目录或 Rust 构建缓存。正式使用建议手动选择一个专用的工作区目录。

工作区数据库位于：

```text
<workspace>\.semanticdrive\metadata.db
```

API 配置位于操作系统应用数据目录，不会写入仓库。
