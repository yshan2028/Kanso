# Kanso 架构设计文档

## 1. 核心设计原则

1. **100% 纯 Rust 全栈原生**：桌面端采用 `iced 0.14` 原生 GPU 渲染（macOS Metal / Windows DirectX / Linux Vulkan），与底层共享同一进程与数据模型，零 FFI 胶水、零浏览器内核开销。
2. **零硬编码、开箱即用**：任何多仓库目录直接拖入即可工作；团队分支基准与 CI 步骤通过 `.kanso.yaml` 或本地 SQLite 偏好配置。
3. **单一内核多端复用**：`kanso-core`、`kanso-storage`、`kanso-ci` 同时驱动桌面应用 `kanso-desktop`、命令行 `kanso`、MCP Server 及远程 `kanso-runner`。

---

## 2. 模块职责

### `kanso-core`
- **VCS 抽象层（`VcsBackend` trait）**：Phase 1 实现并发 `GitCliBackend`（`rayon`），后续可无缝引入 `gix` 极速路径及 Jujutsu（`jj`）后端。
- **多仓发现与护栏**：递归探测 `.git` 边界（自动跳过 `node_modules` / `target` / `dist` 等缓存目录），识别主干脏工作区、游离 HEAD 与合并冲突，提供 `rescue_dirty_trunk` 无损迁移与跨仓批量分支/拉取操作。
- **零手工任务聚合**：将同处于非主干特性分支的仓库自动聚合为 `TaskGroup`，自动推导任务阶段与完成度。
- **实时文件监听**：基于 `notify` + `notify-debouncer-mini`（250ms 防抖），过滤 `.git/objects` 与构建产物目录噪声。

### `kanso-storage`
- **SQLite 持久化（`StorageDb`）**：保存工作区配置与 CI 执行记录。
- **压缩与内容定义分块**：
  - **Zstd**：所有 CI 日志自动经 Zstd 压缩后落库（`log_zstd`），并支撑后续本地时间机快照字典压缩。
  - **FastCDC + BLAKE3**：提供按内容特征切块与 BLAKE3 块级去重指纹（`ContentChunk`），为 Phase 2 增量时间机与 Phase 3 远程 CI 块级增量同步提供统一底座。

### `kanso-ci`
- **流水线探测**：优先读取 `.kanso.yaml`，未配置时自动探测 `Cargo.toml`、`pom.xml`、`build.gradle`、`package.json`、`go.mod`。
- **统一执行器（`CiRunner` trait）**：Phase 1 提供流式 `LocalCiRunner`（Unix `sh -lc` / Windows `cmd /C`）与取消令牌；Phase 3 扩展 `RemoteSshRunner` 与 `RemoteAgentRunner`。

### `kanso-mcp` & `kanso-runner`
- `kanso-mcp`：提供 `kanso` CLI（`scan` / `tasks` / `ci` / `mcp`）及 stdio JSON-RPC 2.0 MCP Server。
- `kanso-runner`：面向远程 Linux/macOS/Windows 构建节点的无头执行器骨架。

### `apps/kanso-desktop`
- 基于 `iced 0.14` + `rfd` 构建的跨平台原生 GPU 控制塔：
  - 侧边栏：工作区选择/添加/移除、视图切换、一键复制 MCP JSON 配置
  - 护栏横幅：主干误改等异常一键救场
  - 全景雷达：汇总指标、搜索筛选、批量建分支/拉主干、高密度多仓状态表
  - 任务进度：自动聚合的跨仓任务卡片与阶段进度条
  - 本机 CI：探测流水线/自定义命令、实时日志控制台、Zstd 压缩历史回放
