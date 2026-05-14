# Semantic Drive AI — 语义智能文件管家

跨平台便携式桌面应用，无需安装，可直接从 U 盘或移动固态硬盘运行。使用自然语言管理和查找存储设备中的文件，完全离线运行，不依赖任何云服务。

## 功能概览

- **自然语言语义搜索** — 用中文自然语言描述查找文件（如"上周修改的财务报表"）
- **自动文件分类** — 按内容将文件归入虚拟文件夹，自动生成标签
- **重复文件检测** — 基于哈希和内容相似度识别重复和相似文件
- **加密安全空间** — AES-256 加密沙盒，保护敏感文件
- **完全离线 AI** — 所有模型本地运行，断网可用

## 技术栈

| 层 | 技术 |
|---|------|
| UI | Tauri v2 + React 19 + TypeScript |
| 后端 | Rust (scanner, AI pipeline, search, vault) |
| Embedding | BGE-small-zh (n-gram 哈希 MVP, ONNX 待集成) |
| 向量搜索 | 余弦相似度 + 关键词混合搜索 |
| LLM | Qwen2 0.5B GGUF (candle 待集成) |
| OCR | PaddleOCR (待集成) |
| 数据库 | SQLite (rusqlite, bundled) |
| 加密 | AES-256-GCM + Argon2id |

## 运行环境

- **Windows 10+** 或 **macOS 12+**
- **Node.js** >= 22.x
- **Rust** >= 1.77 (GNU toolchain: `stable-x86_64-pc-windows-gnu`)
- **MinGW-w64** (Windows) — 通过 WinLibs 或 MSYS2 安装
- **Python 3.10+** (可选, 用于测试脚本)

## 快速开始

### 1. 安装依赖

```bash
# 安装 Rust (GNU toolchain)
rustup default stable-x86_64-pc-windows-gnu

# Windows: 确保 MinGW-w64 在 PATH 中
# 推荐: winget install BrechtSanders.WinLibs.POSIX.UCRT

# 安装 Node.js 依赖
npm install
```

### 2. 开发模式启动

```bash
npm run tauri:dev
```

### 3. 生产构建

```bash
npm run tauri:build
```

构建产物位于 `src-tauri/target/release/bundle/`。

## 项目结构

```
semantic-drive-ai/
├── src/                      # React 前端
│   ├── components/Layout.tsx # 侧边栏 + 布局
│   ├── pages/                # 4 个页面组件
│   │   ├── SmartSearch.tsx   # 智能搜索
│   │   ├── FileClassify.tsx  # 文件分类
│   │   ├── OrganizeSuggestions.tsx # 整理建议
│   │   └── SecureSpace.tsx   # 安全空间
│   └── store/                # Zustand 状态管理
├── src-tauri/                # Rust 后端
│   ├── src/
│   │   ├── lib.rs            # Tauri 命令注册
│   │   ├── main.rs           # 入口
│   │   ├── scanner/mod.rs    # 文件扫描器
│   │   ├── store/mod.rs      # SQLite 元数据存储
│   │   ├── ai/               # AI 管道
│   │   │   ├── mod.rs        # 内容提取调度
│   │   │   ├── extractor.rs  # PDF/DOCX/XLSX/TXT 提取
│   │   │   ├── embedding.rs  # 文本向量化引擎
│   │   │   └── search.rs     # 混合搜索引擎
│   │   ├── classifier.rs     # 文件分类器
│   │   ├── dedup.rs          # 重复文件检测
│   │   └── vault.rs          # 加密保险箱
│   └── Cargo.toml
├── package.json
└── vite.config.ts
```

## 运行测试

```bash
# 前端类型检查
npm run build

# Rust 后端检查
cd src-tauri && cargo check

# Rust 测试 (待添加)
cargo test

# 前端测试 (待添加)
npm test
```

## AI 模型说明

当前 MVP 版本使用轻量级 n-gram 哈希进行文本向量化，无需下载额外模型。

后续版本将集成以下模型（自动下载至 `.semanticdrive/models/`）：

| 模型 | 用途 | 大小 | 格式 |
|------|------|------|------|
| BGE-small-zh | 文本向量化 | ~24MB | ONNX |
| Qwen2-0.5B-Instruct | 查询理解 | ~400MB | GGUF Q4_K_M |
| PaddleOCR-zh | 图片文字识别 | ~30MB | ONNX |

所有模型均为完全离线运行，无需网络连接。

## 注意事项

- 应用仅在所在存储设备范围内扫描文件，不会访问电脑本地磁盘
- 不会修改或破坏原有文件，仅在设备根目录创建 `.semanticdrive` 缓存文件夹
- 首次扫描可能需要数分钟，后续启动仅扫描增量变化
- 加密空间使用 AES-256-GCM 加密，密码不可找回
