# Semantic Drive AI

**Semantic Drive AI** 是一个 API-first、跨平台的 AI 文件管理助手。它基于 Tauri 2、React、TypeScript 和 Rust 构建，使用自然语言帮助用户搜索、理解和整理本地工作区中的文件。

这是原参赛作品的持续演进版本。项目已经从“便携式 U 盘 + 本地大模型”调整为“普通桌面环境 + 云端 AI API + 受控 Agent 文件操作”。当前优先保证 Windows 体验，同时保留 macOS/Linux 的跨平台基础。

## 项目定位

> **一个带 Tool Calling、用户审批、安全边界、操作审计和增量索引的 AI 文件管理 Agent。**

它不是让模型直接执行任意文件命令，而是采用以下受控流程：

```text
用户请求
  -> 文件检索 / RAG 上下文
  -> API 生成回答或结构化工具调用
  -> 前端展示操作计划
  -> 用户逐项或批量确认
  -> Rust 工具层校验 file_id 和工作区边界
  -> 执行文件操作
  -> 更新索引、记录历史并返回结果
```

## 主要能力

### 文件管理

- 选择工作区目录，扫描普通目录、项目目录、外接硬盘或同步目录
- 目录浏览、文件打开、定位到文件所在目录
- 移动、复制、导入、重命名、删除和创建文件夹
- 文件标签：设置、添加、移除和按标签浏览
- 重复文件检测
- 安全空间：使用 AES-256-GCM 加密文件，并通过 Argon2 派生密钥

### 搜索与索引

- 文件名、路径、扩展名、时间范围和标签过滤
- SQLite FTS5 全文搜索
- 轻量确定性 n-gram 本地回退向量
- 可选 OpenAI-compatible Embedding API
- 云端 Embedding 批量重建任务
- 云端向量索引按模型名称隔离，重启后可恢复语义搜索结果
- 文件监听后的单文件增量同步，而不是每次变化都全量扫描

### AI Assistant / Agent

- 使用 OpenAI-compatible Chat Completions API
- 支持原生 JSON Schema Tool Calling
- 兼容不支持 Tool Calling 的服务，通过 `[ACTION:{...}]` 回退
- 支持搜索、打开、移动、复制、重命名、删除、导入、标签和安全空间操作
- 优先使用 `file_id`，避免模型直接生成并执行任意路径
- 高风险文件操作必须经过用户确认
- 批量操作计划预览、逐项确认和批量确认
- 操作历史和部分操作撤销
- 后台任务中心、进度通知、取消和失败重试

## 文件类型边界

项目会对工作区内的普通文件统一建立元数据索引，因此图片、音频、视频和压缩包并不会被完全忽略。它们可以被搜索和管理，但当前内容理解主要面向可提取文本的文件。

| 文件类型 | 文件级管理 | 内容提取 / AI 理解 |
|---|---:|---:|
| TXT、MD、LOG、JSON、XML、HTML | 支持 | 支持 |
| 代码、配置和脚本文件 | 支持 | 支持 |
| PDF | 支持 | 支持文本型 PDF；扫描型 PDF 暂无 OCR |
| DOCX | 支持 | 支持 |
| XLSX、CSV、TSV | 支持 | 支持 |
| XLS | 支持 | 支持基础表格文本提取；复杂格式建议优先使用 XLSX |
| PPT、PPTX | 支持 | 当前暂无演示文稿内容提取 |
| JPG、PNG、GIF、WEBP 等图片 | 支持 | 当前暂无 OCR / 图片理解 |
| MP3、WAV、FLAC 等音频 | 支持 | 当前暂无语音转写 |
| MP4、AVI、MKV、MOV 等视频 | 支持 | 当前暂无视频转写或视觉分析 |
| ZIP、RAR、7Z 等压缩包 | 支持 | 当前不自动解析压缩包内部内容 |

因此当前版本适合展示“文本文件和办公文档的 AI 搜索与整理”，而不是声称已经完成全格式多模态理解。后续可以在不改变 Agent 和索引主架构的情况下增加 OCR、Whisper 转写和视觉模型能力。

## 技术架构

```text
React 19 + TypeScript + Vite + Zustand
                    |
              Tauri IPC
                    |
Rust / Tauri 2 backend
  |          |             |
SQLite   文件系统       API Client
FTS5     Watcher        Chat / Embedding
  |          |             |
  +------ Agent Tool Layer ------+
         file_id 校验
         工作区边界校验
         用户确认
         操作历史
```

### 关键模块

- `src/pages/SmartSearch.tsx`：搜索、目录浏览、标签和文件操作入口
- `src/pages/SmartAssistant.tsx`：对话、Agent 操作确认和任务中心
- `src/store/chatStore.ts`：聊天流式事件、待确认 Action 和前端状态
- `src-tauri/src/ai/api_client.rs`：OpenAI-compatible Chat / Embedding API
- `src-tauri/src/chat.rs`：RAG 上下文、工具 Schema、Action 兼容解析
- `src-tauri/src/store/mod.rs`：SQLite 文件索引、聊天、任务和操作历史
- `src-tauri/src/scanner/mod.rs`：全量扫描、工作区边界和单文件扫描
- `src-tauri/src/watcher.rs`：文件变化到 SQLite 和搜索缓存的增量同步
- `src-tauri/src/lib.rs`：Tauri 命令、安全文件工具和应用装配
- `src-tauri/src/vault.rs`：安全空间和文件加密

## API 配置

应用默认使用 API 模式，不再下载或加载本地大模型。

在设置页面填写：

### Chat / Agent API

- Base URL，例如：`https://api.example.com/v1`
- 模型名称
- API Key

用于：

- 对话
- 文件问答
- Agent 工具调用
- 操作计划生成

### Embedding API

- Base URL
- Embedding 模型
- API Key

用于：

- 云端语义检索
- 批量重建向量索引

未配置 Embedding API 时，应用仍可使用文件名、路径、标签、时间过滤、SQLite FTS5 和本地 fallback 搜索。

项目不会把 API Key 写入源码、Git 或日志。配置保存在操作系统应用数据目录中。

## 安全设计

文件操作是本项目 Agent 设计的重点。模型只负责理解用户意图和提出工具调用，不能直接获得任意命令执行权限。

Rust 后端会校验：

- `file_id` 是否存在于当前工作区索引
- 文件是否仍然存在
- 路径是否位于当前工作区内
- 是否包含绝对路径、Windows 驱动器前缀或路径穿越
- 重命名名称是否包含目录分隔符
- 操作是否经过前端用户确认

执行后的结果会写入操作历史。重命名、移动和部分标签操作支持生成逆操作并撤销；删除、加密等高风险操作不会伪造不安全的自动回滚。

## 数据位置

- 用户配置：操作系统应用数据目录下的 `SemanticDrive/config.json`
- 工作区数据库：`<workspace>/.semanticdrive/metadata.db`
- 云端向量：工作区数据库中的 `cloud_embeddings` 表
- Agent 任务：工作区数据库中的 `agent_tasks` 表
- 操作历史：工作区数据库中的 `action_history` 表
- 安全空间：`<workspace>/.semanticdrive/vault/`
- Rust 编译缓存：`src-tauri/target/`，已被 Git 忽略，不应打包提交

## 开发文档

- [构建与启动指南](BUILD.md)：Windows 开发、打包、首次运行和工作区说明。

当前版本默认通过 OpenAI-compatible API 提供 Chat / Embedding 能力，不再需要下载或启动本地大模型。图片、音频、视频和压缩包仍可作为普通文件管理，但暂未接入 OCR、语音转写、视觉理解或压缩包内部索引。
## 开发环境

- Node.js 22+
- Rust stable
- Windows 10/11、macOS 12+ 或主流 Linux 桌面环境
- Windows 构建需要 WebView2；Tauri 使用系统 WebView2 运行时

```bash
npm install

# 仅启动前端开发服务器
npm run dev

# 启动 Tauri 桌面应用
npm run tauri:dev

# 前端类型检查和生产构建
npm run build

# ESLint
npm run lint

# Rust 编译检查
cd src-tauri
cargo check

# Rust 单元测试
cargo test
```

Windows GNU 工具链如果缺少匹配的 MinGW 运行时 DLL，`cargo test` 可能在测试二进制启动阶段出现 `STATUS_ENTRYPOINT_NOT_FOUND`。这属于本地运行时环境问题；`cargo check` 仍然可以用于验证 Rust 编译。

## 首次使用

1. 启动应用并进入“设置”。
2. 选择一个工作区目录。
3. 填写 Chat / Agent API 的地址、模型和 API Key。
4. 如果需要语义搜索，再填写 Embedding API。
5. 回到“智能搜索”并执行首次扫描。
6. 在“智能助手”中进行自然语言搜索或整理请求。
7. 对涉及文件变化的操作检查计划，确认后再执行。
8. 如需云端语义索引，在“任务中心”启动 Embedding 重建任务。

## 典型使用流程

推荐用一个完整场景验证项目，而不是逐个验证孤立功能：

```text
1. 选择工作区并完成首次扫描
2. 输入“找出最近修改的合同文件”
3. 展示 FTS / 语义搜索结果和文件上下文
4. 输入“把这些文件移动到合同/2026，并添加待归档标签”
5. 展示 JSON Schema Tool Calling 生成的批量计划
6. 用户逐项或批量确认
7. 展示 Rust 安全校验、SQLite 更新和操作历史
8. 撤销移动或标签操作
9. 在系统文件管理器中修改文件，展示 watcher 增量同步
```

项目的重点不是“模型自己执行一切”，而是提供一个真实可控的 AI 文件管理闭环：

```text
AI 理解
+ 检索增强
+ 结构化工具调用
+ 前端审批
+ Rust 安全执行
+ 数据持久化
+ 增量索引
+ 任务与审计
```

## 当前限制与后续方向

当前版本暂不包含：

- 图片 OCR 和视觉内容理解
- 音频 / 视频语音转写
- 扫描型 PDF OCR
- 压缩包内部的自动索引
- 删除操作的完整回收站式回滚
- 多步骤 Agent 的自动观察、重新规划和循环执行

后续可以继续完善：

1. 将 Agent 命令拆分为独立的 `agent/` 和 `commands/` 模块。
2. 增加目录级批量整理计划和更完善的执行队列。
3. 为云端 API 增加并发限制、重试退避和成本统计。
4. 增加 OCR、Whisper 转写和多模态 Embedding 插件。
5. 增加 Windows 回收站、macOS Finder 和 Linux 文件管理器的原生适配。
6. 增加 CI，自动执行前端构建、Lint、Rust 编译和跨平台测试。

## 设计原则

- API-first，但保留本地 FTS 和 fallback 搜索能力
- LLM 不直接执行文件操作
- 所有高风险操作必须经过用户确认
- 所有文件操作都限制在当前工作区
- 优先使用稳定的 `file_id`，而不是信任模型生成的路径
- 大量扫描、解析、索引和 Embedding 工作放到后台线程
- 文件变化采用增量同步，避免频繁全量重建
- 不把 API Key、用户文件内容或敏感信息写入 Git 和日志

## License

MIT
