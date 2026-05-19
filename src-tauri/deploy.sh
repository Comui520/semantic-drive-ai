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

# ── 1+2. Tauri 构建（前端 + Rust 释放 + 嵌入前端）──
echo ""
echo "[1/2] Tauri 构建 (release, 前端嵌入 exe)..."
export PATH="$HOME/.cargo/bin:$MINGW_BIN:$PATH"
cd "$SCRIPT_DIR/.."
npx tauri build --no-bundle

# ── 3. 创建分发文件夹 ──
echo ""
echo "[2/2] 组装分发包..."
rm -rf "$DIST_DIR"
mkdir -p "$DIST_DIR"
mkdir -p "$DIST_DIR/.semanticdrive/models/bge-base-zh"
mkdir -p "$DIST_DIR/.semanticdrive/models/bge-base-en"
mkdir -p "$DIST_DIR/.semanticdrive/models/qwen2.5-7b"
# 保留小模型目录位置，日后需要时直接放文件即可
mkdir -p "$DIST_DIR/.semanticdrive/models/bge-small-zh"
mkdir -p "$DIST_DIR/.semanticdrive/models/qwen2.5-0.5b"

# 复制主程序
cp "src-tauri/target/release/semantic-drive-ai.exe" "$DIST_DIR/"

# 复制 MinGW 运行库
echo "  -> 复制运行库 DLL..."
cp "$MINGW_BIN/libstdc++-6.dll" "$DIST_DIR/" 2>/dev/null || echo "  [警告] libstdc++-6.dll 未找到"
cp "$MINGW_BIN/libgcc_s_seh-1.dll" "$DIST_DIR/" 2>/dev/null || echo "  [警告] libgcc_s_seh-1.dll 未找到"
cp "$MINGW_BIN/libwinpthread-1.dll" "$DIST_DIR/" 2>/dev/null || echo "  [警告] libwinpthread-1.dll 未找到"

# 复制 WebView2Loader.dll（优先用 lib/ 缓存，确保版本正确）
echo "  -> 复制 WebView2Loader.dll..."
if [ -f "$SCRIPT_DIR/lib/WebView2Loader.dll" ]; then
    cp "$SCRIPT_DIR/lib/WebView2Loader.dll" "$DIST_DIR/"
    echo "  ✅ WebView2Loader.dll 已从 lib/ 缓存复制"
else
    echo "  ⚠️  lib/WebView2Loader.dll 未找到，目标机器需要安装 WebView2 Runtime"
    echo "     下载地址: https://developer.microsoft.com/microsoft-edge/webview2/"
fi

# ── 模型复制 ──
echo ""
echo "   打包 AI 模型..."
MODELS_SRC="/d/aiccfuture/models"

copy_model() {
    local name="$1"
    local src="$MODELS_SRC/$name"
    local dst="$DIST_DIR/.semanticdrive/models/$name"
    if [ -d "$src" ] && [ -n "$(ls -A "$src" 2>/dev/null)" ]; then
        cp "$src/"* "$dst/" 2>/dev/null
        echo "  ✅ $name ($(du -sh "$dst" | cut -f1))"
    else
        echo "  ⚠️  $name 目录为空，跳过"
    fi
}

copy_model "bge-base-zh"
copy_model "bge-base-en"

# qwen2.5-7b: 只复制需要的文件，避免重复 GGUF
QWN_DST="$DIST_DIR/.semanticdrive/models/qwen2.5-7b"
if [ -d "$MODELS_SRC/qwen2.5-7b" ]; then
    if [ -f "$MODELS_SRC/qwen2.5-7b/model.gguf" ]; then
        cp "$MODELS_SRC/qwen2.5-7b/model.gguf" "$QWN_DST/"
        echo "  ✅ qwen2.5-7b/model.gguf"
    else
        echo "  ⚠️  qwen2.5-7b/model.gguf 未找到"
    fi
    cp "$MODELS_SRC/qwen2.5-7b/tokenizer.json" "$QWN_DST/" 2>/dev/null || true
else
    echo "  ⚠️  qwen2.5-7b 目录不存在，跳过"
fi

# ── Autorun.inf (Windows AutoPlay 支持) ──
echo ""
echo "   生成 Autorun.inf..."
cat > "$DIST_DIR/../Autorun.inf" << 'INF'
[AutoRun]
Open=SemanticDrive/semantic-drive-ai.exe
Action=打开语义智能文件管家
Icon=SemanticDrive/semantic-drive-ai.exe,0
Label=语义智能文件管家
INF
echo "  ✅ Autorun.inf 已生成 ($DIST_DIR/../Autorun.inf)"
echo "     注意: 新版 Windows 可能因安全策略禁用 AutoRun，"
echo "     用户可能需要手动允许或在通知中选择打开方式"

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
