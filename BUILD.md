# Semantic Drive AI 构建与部署指南

## 目录
- [环境要求](#环境要求)
- [日常开发启动](#日常开发启动)
- [打包分发（U盘便携版）](#打包分发u盘便携版)
- [制作安装程序](#制作安装程序)
- [更新项目](#更新项目)
- [常见问题](#常见问题)

---

## 环境要求

| 工具 | 版本要求 | 用途 |
|------|----------|------|
| Rust | stable (GNU toolchain) | 后端编译 |
| Node.js | >= 18 | 前端构建 |
| MinGW-w64 | POSIX + UCRT | Windows GNU 链接 |
| Git | 任意版本 | 版本管理 |

### 环境检测

打开 Git Bash，运行：

```bash
# Rust
rustc --version
# 应输出: rustc 1.77.2 或更新

# 确认 GNU 工具链
rustup show
# default toolchain 应为 stable-x86_64-pc-windows-gnu

# Node.js
node --version
# 应输出 v18.x 或更新

# MinGW
gcc --version
# 应输出 gcc (x86_64-posix-seh-rev1, Built by Brecht Sanders)
```

如果 `default toolchain` 不是 `gnu`，手动设置：

```bash
rustup default stable-x86_64-pc-windows-gnu
```

---

## 日常开发启动

```bash
# 1. 进入项目目录
cd D:/aiccfuture/semantic-drive-ai

# 2. 启动开发模式（前端热更新 + 后端调试）
npm run tauri:dev
```

首次启动会安装 npm 依赖并编译 Rust 后端。第二次启动会快很多。

**开发者注意事项：**
- 前端代码改完即时生效，无需重启
- Rust 代码改完需要重新编译（`tauri:dev` 会自动触发）
- 后端日志在命令行窗口显示，前端日志在浏览器 DevTools Console 查看
- 如果有 MinGW DLL 报错，运行 `cd src-tauri && cargo build` 会自动复制 DLL

---

## 打包分发（U盘便携版）

生成一个可直接复制到 U 盘运行的文件夹。

### 一键打包

```bash
# 在 Git Bash 中运行
cd src-tauri
bash deploy.sh
```

执行完毕后，产物在 `src-tauri/target/deploy/SemanticDrive/`。

### 目录结构

```
SemanticDrive/
├── semantic-drive-ai.exe    # 主程序，双击运行
├── libstdc++-6.dll          # MinGW 运行库（自动复制）
├── libgcc_s_seh-1.dll       # MinGW 运行库（自动复制）
├── libwinpthread-1.dll      # MinGW 运行库（自动复制）
└── .semanticdrive/models/   # AI 模型（可选，自动下载）
    ├── bge-small-zh/        # 嵌入模型（必需，约 96MB）
    └── qwen2.5-0.5b/        # 语言模型（可选，约 398MB）
```

### 手动打包

如果 `deploy.sh` 不适用（比如没有 Git Bash），按以下步骤手动操作：

```bash
# 1. 构建前端
cd D:/aiccfuture/semantic-drive-ai
npm run build

# 2. 构建 Rust 后端
cd src-tauri
export PATH="$HOME/.cargo/bin:$PATH"
cargo build --release

# 3. 创建分发文件夹
mkdir -p target/deploy/SemanticDrive/.semanticdrive/models/bge-small-zh
mkdir -p target/deploy/SemanticDrive/.semanticdrive/models/qwen2.5-0.5b

# 4. 复制主程序
cp target/release/semantic-drive-ai.exe target/deploy/SemanticDrive/

# 5. 复制 MinGW DLL
MINGW_BIN="/c/Users/Comui/AppData/Local/Microsoft/WinGet/Packages/BrechtSanders.WinLibs.POSIX.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe/mingw64/bin"
cp "$MINGW_BIN/libstdc++-6.dll" target/deploy/SemanticDrive/
cp "$MINGW_BIN/libgcc_s_seh-1.dll" target/deploy/SemanticDrive/
cp "$MINGW_BIN/libwinpthread-1.dll" target/deploy/SemanticDrive/

# 6. 复制模型文件（若已下载）
# 从 src-tauri/target/release/.semanticdrive/models/ 复制到对应目录
```

### 给用户的分发方式

1. 把整个 `SemanticDrive/` 文件夹复制到 U 盘
2. 用户插上 U 盘后，双击 `semantic-drive-ai.exe` 即可运行
3. 首次运行需要联网下载 AI 模型（侧边栏点击下载），约 96MB ~ 500MB
4. 模型下载一次后离线可用

如果想省去用户下载模型的步骤，可以在打包前先在本地下好模型再打包（见下面"内置模型"）。

### 内置模型（省去用户下载）

打包前先启动开发模式，下载好模型，然后：

```bash
# 模型会自动下载到 src-tauri/target/release/.semanticdrive/models/
# deploy.sh 会自动从那里复制到分发包
# 也可以手动放到 src-tauri/.semanticdrive/models/ 目录下
```

---

## 制作安装程序

如果需要制作正式的安装程序（NSIS 或 MSI），推荐使用 **Tauri Bundler**。

### 方法 1：Tauri 自带打包（推荐）

```bash
# 这步会构建 release 版本 + 生成安装程序
# 确保已安装 NSIS（从 https://nsis.sourceforge.io/ 下载）
cd src-tauri
cargo tauri build
```

产物在 `src-tauri/target/release/bundle/`：

```
target/release/bundle/
├── msi/
│   └── SemanticDriveAI_0.1.0_x64_en-US.msi    # Windows Installer
├── nsis/
│   └── SemanticDriveAI_0.1.0_x64-setup.exe     # NSIS 安装程序
└── deb/                                        # Linux 用
```

### 方法 2：自己打包为 ZIP

供不想安装的用户，直接解压使用：

```bash
# 先运行 deploy.sh
cd src-tauri
bash deploy.sh

# 然后压缩分发包
cd target/deploy
zip -r SemanticDrive.zip SemanticDrive/
```

用户只需解压 `SemanticDrive.zip` 到任意位置，运行 `semantic-drive-ai.exe` 即可。

### 安装包内包含

- 主程序（~15MB）
- MinGW 运行库 DLL（~3MB）
- AI 模型（可选，离线下载好后打包可减少用户等待时间）

---

## 更新项目

### 场景 1：你修改了代码

```bash
# 1. 查看改动了什么
git status
git diff

# 2. 暂存并提交
git add <改动的文件>
git commit -m "描述你的改动"

# 3. 重新打包
bash src-tauri/deploy.sh
```

### 场景 2：从头开始在新电脑上跑

```bash
# 1. 克隆（如果远程有）或解压源码
git clone <仓库地址>
# 或者直接复制整个源码文件夹

# 2. 安装 Rust GNU 工具链
rustup default stable-x86_64-pc-windows-gnu

# 3. 安装 Node 依赖
cd semantic-drive-ai
npm install

# 4. 启动
npm run tauri:dev
```

### 场景 3：更新依赖版本

```bash
# 更新前端依赖
npm update

# 更新 Rust 依赖
cd src-tauri
cargo update

# 检查是否有 breaking changes
cargo check
```

### 场景 4：需要重新构建（清理）

```bash
# 清理 Rust 编译缓存（解决奇怪的编译错误）
cd src-tauri
cargo clean
cargo build

# 清理前端缓存
rm -rf node_modules
npm install
```

---

## 常见问题

### Q: 运行 `tauri:dev` 报错 "linker not found"
A: 确保 MinGW 在 PATH 中：
```bash
export PATH="/c/Users/Comui/AppData/Local/Microsoft/WinGet/Packages/BrechtSanders.WinLibs.POSIX.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe/mingw64/bin:$PATH"
```

### Q: 运行时提示缺少 DLL
A: `src-tauri/build.rs` 会自动复制 DLL。如果仍缺，手动复制：
```bash
cp /c/Users/Comui/.../mingw64/bin/libstdc++-6.dll target/debug/
```

### Q: 模型下载失败
A: 项目会自动尝试主站 → 镜像站（hf-mirror.com）切换。如果都失败：
- 检查网络能否访问 huggingface.co
- 手动下载模型文件放到 `.semanticdrive/models/` 对应目录

### Q: 开发时改 Rust 代码需要每次重启？
A: 不需要。`npm run tauri:dev` 会自动监听 Rust 代码变化并重新编译。

### Q: 如何查看更详细的日志？
A: 在 `src-tauri/src/lib.rs` 中找到日志配置，把 `LevelFilter::Info` 改为 `LevelFilter::Debug`。
