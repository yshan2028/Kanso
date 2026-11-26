# Kanso 迭代路线图

## 第一阶段 — 纯 Rust 原生多仓控制塔与本机 CI（当前 MVP）
- [x] Rust Workspace 多 Crate 架构（`kanso-core`、`kanso-storage`、`kanso-ci`、`kanso-mcp`、`kanso-runner`）
- [x] `iced 0.14` 跨平台原生 GPU 桌面端（`kanso-desktop`）
- [x] 并发多仓扫描、实时文件监听与护栏告警（主干误改一键救场）
- [x] 零手工任务聚合视图与跨仓批量分支/拉主干
- [x] 本机 CI 流水线（`.kanso.yaml` + 多语言自动探测）与 Zstd 压缩日志存储
- [x] 标准 stdio MCP Server（`kanso mcp`）

## 第二阶段 — AI 安全网：CoW 零空间沙箱与 Zstd 时间机
- [ ] 每个任务的可视化分支与 Worktree 树视图
- [ ] OS 原生写时复制（macOS APFS `clonefile` / Windows ReFS / Linux Btrfs Reflink）毫秒级沙箱
- [ ] 本地 Zstd 增量时间机（自动留存 AI 试错快照、不污染 `.git`、一键倒带）
- [ ] 提交前安检（大文件拦截、密钥扫描、空闲自动 `.git` 瘦身）

## 第三阶段 — 远程 CI 算力调度与块级增量同步
- [ ] SSH 直连 `RemoteSshRunner` 与无头 `kanso-runner` 守护进程
- [ ] 基于 FastCDC + BLAKE3 + Zstd 的桌面端到构建机块级增量同步
- [ ] 多 Remote 协同（私有 Forgejo/Gitea 高频推送 + 受控上游同步）
- [ ] 合入主干前 CI 门禁校验

## 第四阶段 — 深度 AI 生态与下一代 VCS
- [ ] 扩展 MCP 工具集（快照创建/回滚、跨仓变更摘要、任务验收通知）
- [ ] 通过 `VcsBackend` 可选适配 Jujutsu (`jj`) 仓库
- [ ] GitHub Actions 自动打包发布 macOS / Windows / Linux 原生安装包
