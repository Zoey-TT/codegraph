# codegraph 开源发布前缺口分析

> 基于 `/Users/seven/Documents/codegraph` 现有项目梳理，目标：达到可开源发布（v0.1.0）的质量标准。

---

## 一、项目当前状态速览

| 维度 | 状态 |
|------|------|
| 代码量 | ~21k 行 Rust，61 个源文件 |
| 编译 | ✅ `cargo check` 通过 |
| 测试 | ✅ 133 个单元测试通过（cg-parser 119 + cg-search 12 + cg-server 2） |
| Clippy | 🟡 少量 warning，可快速修复 |
| 文档 | 🟡 README 较完整，缺少 LICENSE、CONTRIBUTING、CHANGELOG 等 |
| CI/CD | ❌ 无 GitHub Actions |

---

## 二、开源发布必备项（P0 — 阻塞项）

> ✅ 表示已完成；🔄 表示进行中；⬜ 表示待处理

### 2.1 法律与许可证

| 缺口 | 说明 | 状态 |
|------|------|------|
| 缺少 LICENSE 文件 | `Cargo.toml` 声明 `MIT OR Apache-2.0`，但仓库根目录没有 `LICENSE-MIT` / `LICENSE-APACHE` | ✅ 已添加 |
| 每个 crate 缺少 license 字段 | 部分 `Cargo.toml` 已继承 workspace，需检查是否全部正确 | ⬜ 待处理 |
| 依赖许可证合规 | 需确认所有依赖许可证兼容（`cargo-deny` 扫描） | ⬜ 待处理 |

### 2.2 代码质量

| 缺口 | 说明 | 状态 |
|------|------|------|
| Clippy warning | cg-server、cg-cli、cg-mcp 有 warning | ⬜ 待处理 |
| 死代码 | `cg-cli::registry::unregister_repo` 未使用 | ⬜ 待处理 |
| Cypher 注入风险 | 已移除 Cypher 查询功能，无需处理 | ✅ 已移除 |
| 错误处理粗糙 | 大量 `anyhow::Result`，缺少结构化错误 | ⬜ 待处理 |
| 缺少 `cargo fmt` 检查 | 需统一格式化 | ⬜ 待处理 |

### 2.3 核心功能验证

| 缺口 | 说明 | 状态 |
|------|------|------|
| 核心 CLI 真实仓库验证 | `analyze / query / context / impact` 需在真实仓库跑通 | ✅ 已验证（codegraph 自身：1573 nodes / 20431 edges） |
| 删除仓库中 `.DS_Store` | 已加入 `.gitignore`，但仍被 git 追踪 | ✅ 已删除并取消追踪 |

### 2.3 核心功能完整性

| 模块 | 当前状态 | 缺口 |
|------|----------|------|
| `cg-cli` analyze | ✅ 完整 | 基本可用 |
| `cg-cli` query/context/impact | ✅ 完整 | 基本可用 |
| `cg-cli` setup | ✅ 完整 | 支持多客户端 |
| `cg-cli` status/list | ✅ 完整 | 基本可用 |
| `cg-cli` clean | 🟡 部分 | `--all` 未实现 |
| `cg-cli` wiki | ❓ 未验证 | 命令存在，实现需检查 |
| `cg-cli` cypher | ❌ 未实现 | 当前版本不支持 Cypher |
| `cg-cli` communities | 🟡 部分 | 需验证可用性 |
| `cg-mcp` query/context/impact/list_repos | ✅ 已实现 | 基本可用 |
| `cg-mcp` detect_changes/rename/route_map/tool_map/shape_check/api_impact | ❌ 未实现 | 仅声明了工具名 |
| `cg-server` /api/query /api/context /api/impact /api/cypher | ✅ 已实现 | 基本可用 |
| `cg-server` /api/repos /api/repos/{name}/status /api/graph/data | 🟡 桩实现 | 返回空/ok |

### 2.4 测试覆盖

| 缺口 | 说明 | 优先级 |
|------|------|--------|
| 无集成测试 | CLI / MCP / Server 没有端到端测试 | P1 |
| cg-cli 无测试 | 627 行 main.rs 无测试 | P1 |
| cg-mcp 无测试 | 413 行 server 无测试 | P1 |
| cg-core 测试不足 | 内存图/增量更新测试少 | P2 |
| cg-graph 测试不足 | 缺少持久化后端相关测试 | P2 |
| 缺少 fixture | 只有 3 个 fixture 文件 | P2 |

### 2.5 文档

| 缺口 | 说明 | 优先级 |
|------|------|--------|
| README 示例不足 | 缺少真实使用截图/输出示例 | P2 |
| 缺少 CONTRIBUTING.md | 开源必备 | P1 |
| 缺少 CHANGELOG.md | 版本记录 | P2 |
| 缺少 SECURITY.md | 安全报告流程 | P2 |
| 缺少 ARCHITECTURE.md | 架构说明 | P2 |
| API 文档不全 | `cargo doc` 覆盖率待检查 | P2 |
| 缺少 examples/ | 示例代码 | P2 |

### 2.6 工程化

| 缺口 | 说明 | 优先级 |
|------|------|--------|
| 无 CI/CD | 缺少 `.github/workflows/ci.yml` | P1 |
| 无发布脚本 | 缺少 cargo publish / release 流程 | P2 |
| 无 deny.toml | 依赖审计 | P2 |
| 版本号 | 仍为 `0.1.0`，需确认是否作为 v0.1.0 发布 | P2 |
| `.DS_Store` 在仓库中 | 应删除并加入 `.gitignore` | P1 |
| `repository` 是占位符 | `https://github.com/your-org/codegraph` | P1 |

---

## 三、建议修复优先级

### P0（阻塞发布）
1. 添加 `LICENSE-MIT` 和 `LICENSE-APACHE`
2. 删除仓库中的 `.DS_Store`
3. 确认核心 CLI 命令（analyze/query/context/impact）能跑通真实仓库

### P1（发布前应尽量完成）
5. 修复所有 Clippy warning
6. 删除/使用死代码
7. 补 CLI / MCP / Server 的集成测试
8. 实现或隐藏未完成的命令（clean --all、wiki 等）
9. 添加 GitHub Actions CI（build / test / clippy / fmt）
10. 添加 `CONTRIBUTING.md`
11. 修正 `repository` URL
12. 添加 `CHANGELOG.md`（初始版本）

### P2（发布后持续优化）
13. 添加 `ARCHITECTURE.md`
14. 添加 `examples/`
15. 依赖审计（`cargo-deny`）
16. 提升错误处理结构化程度
17. 补全 MCP 剩余工具
18. 补全 Server 桩端点
19. 提升 `cargo doc` 覆盖率

---

## 四、推荐的最小发布路径

如果目标是**尽快达到可开源发布**，建议只做 P0 + P1：

```
Week 1: 法律/工程化/质量
  - LICENSE 文件
  - .gitignore 修复
  - CI workflow
  - Clippy 清零

Week 2: 功能与测试
  - 核心 CLI 端到端测试
  - MCP 集成测试
  - 隐藏/实现未完成命令
  - CONTRIBUTING / CHANGELOG

Week 3: 发布准备
  - README 润色 + 示例
  - cargo publish 预演
  - Git tag v0.1.0
```

---

## 五、当前最大风险

1. **无测试的核心入口**：`cg-cli` 600+ 行代码无单元测试。
2. **许可证文件缺失**：开源仓库不可接受。
3. **未完成的工具暴露在 CLI 中**：`wiki`、`communities` 可能体验不完整，影响第一印象。

---

*分析时间：2026-06-17*
