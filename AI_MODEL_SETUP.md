# AI 模型安装指南

本应用需要下载 AI 模型才能实现真正的语义搜索和查询理解。以下所有模型均需在首次使用时配置好。

**模型总大小**: ~500 MB
**目标目录**: `semantic-drive-ai/models/` （相对于可执行文件所在路径）

---

## 1. BGE-small-zh 嵌入模型（必需）

用于将文本转换为向量，实现语义搜索和智能分类。

| 项 | 值 |
|---|-----|
| 大小 | ~25 MB |
| 格式 | GGUF (Q4_K_M 量化) |
| 来源 | Hugging Face |

**下载方式 A — 自动下载（推荐）**: 首次启动应用时，会自动从 Hugging Face 下载。需要网络连接，下载完成后断网可用。

**下载方式 B — 手动下载**: 如果自动下载失败，请执行：

**Windows PowerShell**（管理员）:
```powershell
cd D:\aiccfuture\semantic-drive-ai
mkdir -p .semanticdrive\models\bge-small-zh
# 从 Hugging Face 下载 BGE-small-zh GGUF 量化模型
Invoke-WebRequest -Uri "https://huggingface.co/ChristianAzinn/bge-small-zh-v1.5-Q4_K_M-GGUF/resolve/main/bge-small-zh-v1.5-q4_k_m.gguf" -OutFile ".semanticdrive\models\bge-small-zh\model.gguf"
```

**Git Bash/Linux**:
```bash
cd /d/aiccfuture/semantic-drive-ai
mkdir -p .semanticdrive/models/bge-small-zh
curl -L -o .semanticdrive/models/bge-small-zh/model.gguf \
  "https://huggingface.co/ChristianAzinn/bge-small-zh-v1.5-Q4_K_M-GGUF/resolve/main/bge-small-zh-v1.5-q4_k_m.gguf"
```

**验证**: 文件大小应为 ~25 MB，SHA256 校验在下载后自动完成。

---

## 2. Qwen3-0.6B LLM 模型（推荐）

用于理解用户的自然语言查询（如"上周修改的财务报表"→ 提取关键词、时间、类型）。

| 项 | 值 |
|---|-----|
| 大小 | ~400 MB |
| 格式 | GGUF Q4_K_M 量化 |
| 来源 | Hugging Face |

**下载**:

**Windows PowerShell**:
```powershell
cd D:\aiccfuture\semantic-drive-ai
mkdir -p .semanticdrive\models\qwen3-0.6b
Invoke-WebRequest -Uri "https://huggingface.co/Qwen/Qwen3-0.5B-GGUF/resolve/main/qwen3-0_5b-q4_k_m.gguf" -OutFile ".semanticdrive\models\qwen3-0.6b\model.gguf"
```

**Git Bash**:
```bash
cd /d/aiccfuture/semantic-drive-ai
mkdir -p .semanticdrive/models/qwen3-0.6b
curl -L -o .semanticdrive/models/qwen3-0.6b/model.gguf \
  "https://huggingface.co/Qwen/Qwen3-0.5B-GGUF/resolve/main/qwen3-0_5b-q4_k_m.gguf"
```

**验证**: 文件大小应为 ~400 MB。

---

## 3. PaddleOCR 模型（可选）

用于从图片中提取文字（如扫描的文档、截图等）。

| 项 | 值 |
|---|-----|
| 大小 | ~50 MB |
| 格式 | ONNX |
| 来源 | Hugging Face / GitHub |

**下载**:

**Windows PowerShell**:
```powershell
cd D:\aiccfuture\semantic-drive-ai
mkdir -p .semanticdrive\models\paddle-ocr
Invoke-WebRequest -Uri "..." -OutFile ".semanticdrive\models\paddle-ocr\*.onnx"
```

（具体 URL 根据集成的 OCR 引擎确定，详见技术实现。）

---

## 模型目录结构

最终目录结构应为：

```
.semanticdrive/
└── models/
    ├── bge-small-zh/
    │   └── model.gguf              # BGE-small-zh 嵌入模型 (~25 MB)
    ├── qwen3-0.6b/
    │   └── model.gguf              # Qwen3 LLM 模型 (~400 MB)
    └── paddle-ocr/
        └── *.onnx                  # PaddleOCR ONNX 模型 (~50 MB)
```

---

## 常见问题

**Q: 下载速度慢怎么办？**
在终端下载通常比浏览器快。如果极慢，可以尝试使用代理或镜像站。

**Q: 下载到一半断了怎么办？**
删除不完整的文件重新下载。应用在加载时会验证模型文件完整性。

**Q: 没有这些模型，应用能运行吗？**
能。应用会自动降级为关键词搜索，但语义搜索和 LLM 查询理解功能不可用。

**Q: 模型文件能放在 U 盘里使用吗？**
可以。模型文件跟随应用目录，复制到 U 盘即可带到其他电脑使用。

**Q: 如何验证模型文件是否正确？**
应用启动时会自动进行完整性检查。文件大小不符或损坏时会提示重新下载。
