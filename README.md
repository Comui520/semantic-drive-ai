# Semantic Drive AI

Semantic Drive AI 是一个 **API-first、跨平台的 AI 文件管理助手**。它运行在 Windows、macOS 和 Linux 的 Tauri 桌面环境中，用自然语言帮助用户搜索、理解和整理文件。

> 这是原参赛作品的持续演进版本。旧版本强调“U 盘便携 + 完全离线模型”；当前版本改为优先使用云端 API，并通过可选工作区适配普通电脑、外接硬盘和网络同步目录。

## 当前定位

- **跨平台文件管理助手**：文件扫描、目录浏览、全文检索、分类、去重、标签和加密空间
- **API 优先**：聊天和语义能力通过 OpenAI 兼容接口接入，可使用 DeepSeek、SiliconFlow、OpenAI-compatible gateway 或自建服务
- **Agent 化交互**：助手可以搜索文件并生成移动、复制、重命名、删除、打标签、加密等操作计划；所有变更操作都必须经过用户确认
- **安全边界**：Agent 只处理选定工作区内的 `file_id`，不信任模型生成的文件路径，不允许通过 `..` 或绝对路径越界
- **可渐进增强**：没有嵌入 API 时仍可使用 SQLite FTS 和轻量 n-gram 回退搜索；不再下载或加载数百 MB 的本地大模型

## 已完成的本轮重构

1. 移除 Candle、Tokenizer、Hugging Face 模型下载和本地 Qwen/BGE 加载链路，减少编译复杂度和运行时占用。
2. 聊天改为 API-only；API 未配置时给出明确设置提示，不再“静默回退到不存在的本地模型”。
3. 默认配置切换为 API-first，配置迁移兼容旧版 `config.json`。
4. 设置页支持填写聊天 API、嵌入 API，并可跨平台选择工作区目录。
5. 修复项目目录膨胀问题：`src-tauri/target` 是编译缓存，当前约 15.9 GB，不属于源码交付物，已由 ESLint 忽略且 `.gitignore` 已覆盖。
6. 强化 Agent 文件操作：优先使用 `file_id`；校验工作区边界、绝对路径、Windows 驱动器前缀、文件名和索引路径。
7. 修复 Action JSON 解析：字符串中的 `{}` 不会再破坏动作标记的解析。
8. 保留增量扫描、SQLite 元数据、FTS5、目录浏览、分类、去重、标签和安全空间等原有能力。

## 技术栈

| 层 | 技术 |
|---|---|
| 桌面壳 | Tauri 2 |
| 前端 | React 19 + TypeScript + Vite + Zustand |
| 后端 | Rust |
| 数据库 | SQLite / rusqlite + FTS5 |
| 内容提取 | TXT、Markdown、代码、PDF、DOCX、XLSX |
| 云端 AI | OpenAI-compatible Chat Completions / Embeddings |
| 本地回退 | 关键词、FTS5、轻量确定性 n-gram 向量 |
| 加密 | AES-256-GCM + Argon2 |

## 开发环境

- Node.js 22+
- Rust stable
- Windows 10/11、macOS 12+ 或主流 Linux 桌面环境
- Windows 构建需要 WebView2；Tauri 会使用系统 WebView2

```bash
npm install
npm run dev          # 仅启动前端
npm run tauri:dev    # 启动桌面应用
npm run build        # 前端类型检查 + 生产构建
npm run lint         # ESLint
cd src-tauri
cargo check          # Rust 编译检查
cargo test           # Rust 单元测试（Windows GNU 环境可能需要匹配的运行时 DLL）
```

## 首次使用

1. 运行应用，进入“设置”。
2. 选择一个工作区目录。工作区可以是用户目录、项目目录、外接硬盘目录或同步盘目录。
3. 填写聊天 API：
   - Base URL 形如 `https://api.example.com/v1`
   - 模型填写服务商提供的模型名
   - 填写 API Key
4. 如需语义搜索，再填写嵌入 API。未填写时仍能使用关键词和全文搜索。
5. 回到“智能搜索”并扫描。首次扫描提取文件元数据和文本，后续扫描只处理新增或修改文件。
6. 在“智能助手”中提出请求。涉及文件变更时，先检查待执行操作，再逐项或批量确认。

## Agent 设计

当前 Agent 采用“**模型负责理解与提出计划，应用负责工具执行和安全校验**”的边界：

```text
用户请求
  -> 意图识别 / RAG 文件上下文
  -> API 生成计划与 [ACTION:{...}] 工具调用
  -> 前端展示待确认动作
  -> 用户确认
  -> Rust 工具层校验 file_id、工作区边界和参数
  -> 执行文件操作、更新 SQLite、返回审计结果
```

当前工具包括：

- 搜索和打开文件
- 移动、复制、导入、重命名、删除
- 设置、添加、移除标签
- 添加到安全空间
- 跳转到分类和去重页面

下一阶段可以将文本动作标记升级为正式的 OpenAI tool calling / JSON Schema，并增加操作历史、撤销、批量计划预览、失败重试和后台任务队列。

## 数据与配置位置

- 用户配置：操作系统应用数据目录下的 `SemanticDrive/config.json`
- 工作区索引：工作区下的 `.semanticdrive/metadata.db`
- 安全空间：工作区下的 `.semanticdrive/vault/`
- Rust 编译缓存：`src-tauri/target/`，不应提交到 Git 或打包分发

## 代码结构

```text
src/
  components/       UI 组件、文件浏览器、欢迎引导
  pages/            搜索、助手、分类、整理、安全空间、设置
  store/            Zustand 状态
  types/            前后端配置类型
src-tauri/src/
  scanner/          扫描器、工作区边界和文件监听
  store/            SQLite 元数据、标签、聊天记录、配置
  ai/               API 客户端、回退嵌入、查询解析、混合搜索、文本提取
  chat.rs           Agent 提示词、动作协议、RAG 上下文
  lib.rs            Tauri 命令、安全文件工具和应用装配
  vault.rs          加密安全空间
```

## 设计原则

- 不把 API Key 写入源码、Git 或日志
- 不让 LLM 直接执行文件操作；必须经过用户确认和 Rust 校验
- 不依赖本地大模型作为启动前置条件
- 不把应用安装目录自动当成用户工作区；优先使用用户选择的目录
- 大量文件操作放到后台线程，UI 只接收进度事件
- 索引更新采用增量策略，避免每次启动完整扫描
