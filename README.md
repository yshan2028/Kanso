# Kanso (簡素)

**100% 纯 Rust 原生 GPU 渲染 · AI 时代多仓工程控制塔与本地/远程 CI 工作台**

[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](./LICENSE)

---

## 为什么需要 Kanso？

传统 Git 客户端是为人类单仓库、低频手工提交设计的。在 AI Coding Agent（Antigravity / Claude Code / Cursor / Codex）高频并发改代码的时代，多仓研发面临全新痛点：
- 几十个微服务和前端仓库，各自停在哪个分支、偏离主干多远、有没有未提交或未推送改动？
- 跨多个仓库推进同一个需求时，没有直观的任务进度视图，难道要靠人手填表？
- 本地高频验证依赖慢、推 GitHub 高频提交容易触发风控封号，如何灵活调度本机或私有服务器跑 CI？
- AI 高频试错如何高压缩留档、零空间开销沙箱隔离，又不污染正式 `.git` 历史？

**Kanso（簡素）** 采用 **100% 纯 Rust 全栈架构**（底层引擎 + `iced` 原生 GPU 渲染桌面端 + CLI + MCP Server），**零 Electron、零 WebView、零 Node.js 运行时**，一套代码原生编译至 macOS（Metal）、Windows（DirectX）与 Linux（Vulkan）。

---

## 核心能力

- **一键导入父目录 & 多仓全景雷达**  
  使用 `rayon` 并发递归发现目录下全部 Git 仓库与 Worktree，配合 `notify` 实时监听文件变化。一屏掌控当前分支、主干基准、Ahead/Behind 偏移、远程同步、脏文件统计与最近提交。
- **零手工任务进度聚合**  
  自动按活跃特性分支把跨仓工作聚合成任务卡片，自动推导任务阶段（`开发改动中` / `分支进行中` / `待验或待推送` / `可合入主干`）与进度百分比，无需任何人工填表。
- **AI 误操作护栏**  
  实时监测直接在主干分支上改脏代码、Detached HEAD、合并冲突，支持**一键救场：将未提交改动无损迁移到新分支**，以及跨仓批量建分支、安全拉取主干。
- **Zstd + FastCDC + BLAKE3 存储与压缩底座**  
  `kanso-storage` 内置 SQLite 状态库与 Zstd 日志实时压缩，并提供 FastCDC 内容定义分块 + BLAKE3 块级指纹底座，支撑后续本地增量时间机快照与远程 CI 块级同步。
- **本机 CI 流水线（支持自动探测）**  
  优先读取 `.kanso.yaml`，未配置时自动识别 Rust（`Cargo.toml`）、Java Maven（`pom.xml`）、Gradle、Node（`package.json`）、Go（`go.mod`）项目，实时流式输出构建日志并支持一键中止与历史回放。
- **原生 MCP Server（AI 协同接口）**  
  通过标准 stdio JSON-RPC 2.0 暴露 `kanso_scan_workspace`、`kanso_list_tasks`、`kanso_run_local_ci`，让 AI Agent 自己查多仓状态并触发本地验证。

---

## 项目架构

```text
kanso/
├── crates/
│   ├── kanso-core      # 多仓并发扫描、VcsBackend 抽象、任务自动聚合、护栏检查、文件监听
│   ├── kanso-storage   # SQLite 持久化、Zstd 压缩、FastCDC + BLAKE3 内容分块
│   ├── kanso-ci        # 流水线探测、CiRunner 抽象、流式 LocalCiRunner
│   ├── kanso-mcp       # CLI (`kanso`) + stdio JSON-RPC 2.0 MCP Server
│   └── kanso-runner    # 远程构建机无头 Agent 骨架
├── apps/
│   └── kanso-desktop   # 100% 纯 Rust 原生 GPU 桌面端（iced 0.14 + rfd）
└── docs/
    ├── ARCHITECTURE.md # 架构设计与存储压缩方案
    └── ROADMAP.md      # Phase 1–4 迭代路线图
```

---

## 快速开始

### 环境要求
- **Rust** (`1.88+`)
- **Git** (`2.30+`)

### 1. 启动纯 Rust 原生桌面端

```bash
cargo run -p kanso-desktop
```

### 2. 运行 CLI 与 MCP Server

```bash
# 扫描任意包含多个 Git 仓库的目录
cargo run -p kanso-mcp -- scan /path/to/your/workspace

# 查看自动聚合的跨仓任务进度
cargo run -p kanso-mcp -- tasks /path/to/your/workspace

# 在指定仓库运行自动探测或 .kanso.yaml 本机 CI
cargo run -p kanso-mcp -- ci /path/to/repo

# 启动 stdio MCP Server 供 AI Coding Agent 接入
cargo run -p kanso-mcp -- mcp
```

### 3. 运行全工作区测试

```bash
cargo test --workspace
```

---

## 配置说明（`.kanso.yaml`，完全可选）

在工作区根目录或单仓根目录放置 `.kanso.yaml`：

```yaml
trunk_branch: develop
ignore_dirs:
  - archive
pipelines:
  - name: 编译与单元测试
    steps:
      - name: 编译检查
        command: mvn -q test-compile
      - name: 单元测试
        command: mvn -q test
```

未提供 `.kanso.yaml` 时，Kanso 会自动识别主干分支（`main` / `master` / `develop` / `dev`）与各语言常用构建命令，开箱即用。

---

## 开源协议

[Apache License 2.0](./LICENSE)
