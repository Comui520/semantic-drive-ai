# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**Semantic Drive AI (语义智能文件管家)** — a portable, cross-platform desktop app that lets users manage and search files on removable storage (USB/SSD) using natural language, entirely offline.

The project is located at `semantic-drive-ai/`.

## Build Environment (Windows + Git Bash)

Rust uses the **GNU toolchain** (not MSVC) because MSVC build tools are not available in this environment.

```bash
# Required PATH setup before any cargo command:
export PATH="$HOME/.cargo/bin:$PATH"
# MinGW-w64 must be in PATH:
export PATH="/c/Users/Comui/AppData/Local/Microsoft/WinGet/Packages/BrechtSanders.WinLibs.POSIX.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe/mingw64/bin:$PATH"
```

The Rust default toolchain is set to `stable-x86_64-pc-windows-gnu`.

## Commands

```bash
# Development (run from semantic-drive-ai/)
cd semantic-drive-ai
npm run tauri:dev      # Start Tauri dev server

# TypeScript type check
npm run build          # tsc -b && vite build

# Rust backend check
cd semantic-drive-ai/src-tauri
cargo check

# Production build (portable executables)
npm run tauri:build

# Testing (not yet configured)
npm test               # Frontend (Jest + Testing Library)
cargo test             # Rust backend
```

## Architecture

```
UI (React 19/TS) ← IPC → Rust Backend (Tauri v2)
├── scanner/       # File system scanner (walkdir)
├── store/         # SQLite metadata store (rusqlite, bundled)
├── ai/
│   ├── extractor/ # PDF (lopdf), DOCX (zip+xml), XLSX (calamine), TXT
│   ├── embedding/ # N-gram hashing (MVP) → BGE-small-zh ONNX (planned)
│   └── search/    # Hybrid: cosine similarity + keyword matching
├── classifier.rs  # Extension-based + content keyword classification
├── dedup.rs       # BLAKE3 hash-based exact duplicate detection
└── vault.rs       # AES-256-GCM + Argon2id encrypted file storage
```

Frontend: 4 pages (SmartSearch, FileClassify, OrganizeSuggestions, SecureSpace), glassmorphism UI, Zustand state management, Tailwind CSS v4.

## Key Constraints

- **Fully offline**: All AI models bundled or auto-downloaded
- **Portable**: Runs from removable media, no installation
- **Storage-scoped**: Only indexes files on the device where the app resides
- **Non-destructive**: Creates only `.semanticdrive/` cache folder, never modifies originals
- **App size < 2GB** with models
- **Chinese-first**: NLP must handle Chinese natural language

## Frontend (React + TypeScript)

- `src/components/Layout.tsx` — Sidebar with 4 nav items, theme toggle
- `src/pages/` — SmartSearch, FileClassify, OrganizeSuggestions, SecureSpace
- `src/store/` — Zustand stores (themeStore, appStore)
- Tailwind CSS v4 with custom theme colors (deepsea-*, silver-*, accent-cyan, accent-teal)
- Glassmorphism: `.glass` and `.glass-light` utility classes

## Backend Rust Module Map

- `scanner::get_device_root()` — Returns the directory where the exe resides
- `scanner::scan_directory(root)` — WalkDir-based recursive file listing
- `scanner::compute_hash(path)` — BLAKE3 streaming hash
- `store::MetadataStore` — SQLite CRUD for file metadata
- `ai::extract_text(path, ext)` — Dispatches to format-specific extractors
- `ai::search::SearchEngine` — Embeds query, computes cosine similarity
- `classifier::classify_files(entries)` — Returns categories + tags
- `dedup::find_duplicates(entries, root)` — Groups by hash, returns waste
- `vault::*` — AES-256-GCM encrypt/decrypt with Argon2id key derivation

All Tauri commands are registered in `lib.rs::run()`.
