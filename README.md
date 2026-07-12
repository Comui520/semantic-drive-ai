# Semantic Drive AI

**语义智能文件管家** — 运行在 U 盘上的 AI Agent。不用安装，双击即用。语义搜索找到文件，AI Agent 替你操作。

[![Rust](https://img.shields.io/badge/Rust-1.7+-orange.svg)](https://www.rust-lang.org)
[![Tauri](https://img.shields.io/badge/Tauri-v2-blue.svg)](https://tauri.app)
[![React](https://img.shields.io/badge/React-19-61dafb.svg)](https://react.dev)

## 核心能力

| 功能 | 说明 |
|------|------|
| **语义搜索** | BGE-M3 理解查询意图 + FTS5 全文索引，搜"上个月PPT"能找到 final-v3.pptx |
| **AI Agent** | 15 种文件操作——移动、重命名、标签、加密，对话中一句话执行 |
| **智能分类** | AI 按类型+语义自动归类。BLAKE3 哈希精确去重 |
| **安全空间** | AES-256-GCM 加密，Session Key 方案让 Agent 加密无需反复输密码 |
| **100% 本地** | Candle 纯 Rust 推理，零网络请求，数据不离 U 盘 |
| **API 模式** | 可选 SiliconFlow + DeepSeek V4，自动降级本地 |
| **即插即用** | 绿色便携，单文件不到 50MB，不写注册表 |

## 技术栈

| 层 | 技术 |
|------|------|
| 前端 | React 19 · TypeScript · Tailwind CSS · Zustand |
| IPC | Tauri v2 |
| Agent 调度 | 意图检测 · RAG 注入 · Action 解析 · ID→文件名映射 |
| AI 引擎 | Candle (本地) · SiliconFlow BGE-M3 · DeepSeek V4 (API) |
| 存储 | SQLite + FTS5 · BLAKE3 哈希 · AES-256-GCM |

## 快速开始

```bash
# 开发模式
cd semantic-drive-ai
npm install
npm run tauri:dev

# 生产构建
bash src-tauri/deploy.sh
```

需要 Node.js 18+、Rust 1.7+（GNU toolchain）、MinGW-w64。

## 项目结构

```
├── semantic-drive-ai/         # 主程序
│   ├── src/                   # React 前端
│   │   ├── pages/             # 智能搜索 · AI助手 · 分类 · 安全空间 · 设置
│   │   └── store/             # Zustand 状态管理
│   └── src-tauri/src/         # Rust 后端
│       ├── ai/                # embedding · llm · search · api_client
│       ├── store/             # SQLite · config
│       └── lib.rs             # Tauri 命令注册 · Agent 路由
├── pre/                       # 决赛 PPT + 演讲稿 + 图片
└── docs/                      # 商业方案 · 视频脚本
```

## 文档

- [决赛演讲稿](pre/稿子-决赛.md) — 7 分钟讲解 + 18 题评委 Q&A
- [商业方案](docs/商业方案.md) — 市场分析、三层盈利模型、推广策略
- [视频录制方案](docs/plan/视频录制方案.md) — 3 分钟 Agent 演示脚本

## License

MIT
