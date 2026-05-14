#!/usr/bin/env bash
# ==============================================================
#  Semantic Drive AI — 一键部署脚本
#  构建 release 版本，自动复制模型文件和运行库
#  输出到 target/deploy/SemanticDrive/ 可直接压缩分发
# ==============================================================
set -e

MINGW_BIN="/c/Users/Comui/AppData/Local/Microsoft/WinGet/Packages/BrechtSanders.WinLibs.POSIX.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe/mingw64/bin"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
DIST_DIR="$SCRIPT_DIR/target/deploy/SemanticDrive"

echo "=========================================="
echo " Semantic Drive AI — 部署打包"
echo "=========================================="

# ── 1. 构建前端 ──
echo ""
echo "[1/4] 构建前端..."
cd "$SCRIPT_DIR/.."
npm run build

# ── 2. 构建 Rust 后端 ──
echo ""
echo "[2/4] 构建 Rust 后端 (release)..."
export PATH="$HOME/.cargo/bin:$MINGW_BIN:$PATH"
cd "$SCRIPT_DIR"
cargo build --release

# ── 3. 创建分发文件夹 ──
echo ""
echo "[3/4] 组装分发包..."
rm -rf "$DIST_DIR"
mkdir -p "$DIST_DIR"
mkdir -p "$DIST_DIR/.semanticdrive/models/bge-small-zh"
mkdir -p "$DIST_DIR/.semanticdrive/models/qwen2.5-0.5b"

# 复制主程序
cp "target/release/semantic-drive-ai.exe" "$DIST_DIR/"

# 复制 MinGW 运行库
echo "  -> 复制运行库 DLL..."
cp "$MINGW_BIN/libstdc++-6.dll" "$DIST_DIR/" 2>/dev/null || echo "  [警告] libstdc++-6.dll 未找到"
cp "$MINGW_BIN/libgcc_s_seh-1.dll" "$DIST_DIR/" 2>/dev/null || echo "  [警告] libgcc_s_seh-1.dll 未找到"
cp "$MINGW_BIN/libwinpthread-1.dll" "$DIST_DIR/" 2>/dev/null || echo "  [警告] libwinpthread-1.dll 未找到"

# ── 4. 复制模型文件 ──
echo ""
echo "[4/4] 打包 AI 模型..."
MODELS_CACHE="$DIST_DIR/.semanticdrive/models"

# 查找已下载的模型（按优先级）
find_model_dir() {
    local model_name="$1"
    # 1. exe 同目录 (release 运行过)
    local release_dir="$SCRIPT_DIR/target/release/.semanticdrive/models/$model_name"
    # 2. exe 同目录 (debug 运行过)
    local debug_dir="$SCRIPT_DIR/target/debug/.semanticdrive/models/$model_name"
    # 3. 源目录 (手动放置)
    local source_dir="$SCRIPT_DIR/.semanticdrive/models/$model_name"

    if [ -d "$release_dir" ] && [ -n "$(ls -A "$release_dir" 2>/dev/null)" ]; then
        echo "$release_dir"
    elif [ -d "$debug_dir" ] && [ -n "$(ls -A "$debug_dir" 2>/dev/null)" ]; then
        echo "$debug_dir"
    elif [ -d "$source_dir" ] && [ -n "$(ls -A "$source_dir" 2>/dev/null)" ]; then
        echo "$source_dir"
    fi
}

# BGE 模型（必需）
BGE_DIR=$(find_model_dir "bge-small-zh")
if [ -n "$BGE_DIR" ]; then
    cp "$BGE_DIR/"* "$MODELS_CACHE/bge-small-zh/" 2>/dev/null
    echo "  ✅ BGE-small-zh 已打包 ($(du -sh "$MODELS_CACHE/bge-small-zh" | cut -f1))"
else
    echo "  ⚠️  BGE 模型未找到！"
    echo "     请先在 app 中下载 BGE 模型，或手动下载后放入："
    echo "     src-tauri/.semanticdrive/models/bge-small-zh/"
    echo "     需要文件: config.json, tokenizer.json, model.safetensors"
fi

# Qwen LLM 模型（可选，没有也能用）
QWEN_DIR=$(find_model_dir "qwen2.5-0.5b")
if [ -n "$QWEN_DIR" ]; then
    cp "$QWEN_DIR/"* "$MODELS_CACHE/qwen2.5-0.5b/" 2>/dev/null
    echo "  ✅ Qwen2.5 已打包 ($(du -sh "$MODELS_CACHE/qwen2.5-0.5b" | cut -f1))"
else
    echo "  ℹ️  Qwen LLM 模型未打包（可选，搜索功能不受影响）"
fi

# ── 完成 ──
echo ""
echo "=========================================="
echo " ✅ 部署包已创建:"
echo "    $DIST_DIR"
echo ""
echo "    目录结构:"
du -sh "$DIST_DIR"/*
echo ""
echo "    总大小: $(du -sh "$DIST_DIR" | cut -f1)"
echo ""
echo "    使用方式：直接压缩为 ZIP 或复制到 U 盘即可分发"
echo "    接收方插上 U 盘运行 SemanticDrive.exe 即可"
echo "=========================================="
