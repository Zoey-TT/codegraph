# CodeGraph

> 代码知识图谱 — 将代码库转化为可查询、可搜索、可推理的智能图结构。

CodeGraph 是一个用 Rust 实现的代码知识图谱构建工具。它通过多语言 AST 解析、内存图存储和全文搜索，将代码库转化为结构化的知识图谱，支持符号检索、上下文查看、影响分析等查询能力。

[![CI](https://github.com/your-org/codegraph/actions/workflows/ci.yml/badge.svg)](https://github.com/your-org/codegraph/actions/workflows/ci.yml)

## 特性

- **多语言 AST 解析**：基于 Tree-sitter 支持 Rust、TypeScript/JavaScript、Python、Go、Java、C/C++、C#
- **知识图谱构建**：自动提取符号（函数、类、接口、结构体等）及其关系（调用、继承、导入、包含等）
- **符号检索**：基于名称的全文搜索
- **符号上下文**：查看符号的调用方、被调用方、成员和导入
- **影响分析**：沿调用链分析修改某个符号的爆炸半径
- **MCP 服务器**：提供 `query`、`context`、`impact`、`list_repos` 四个工具
- **HTTP API**：基于 Axum 的 REST API

## 架构

项目采用 Cargo Workspace 管理多 crate 架构：

```
crates/
├── cg-common/   # 共享类型、常量、工具函数
├── cg-core/     # 内存知识图谱模型
├── cg-parser/   # 文件扫描、Tree-sitter 解析、符号提取
├── cg-graph/    # 图存储适配器
├── cg-search/   # 全文检索
├── cg-mcp/      # MCP 服务器实现
├── cg-cli/      # CLI 入口
└── cg-server/   # HTTP API 服务
```

## 快速开始

### 前置要求

- [Rust](https://rustup.rs/) 1.81.0 或更高版本

### 安装

```bash
# 从源码安装
cargo install --path crates/cg-cli

# 或使用发布二进制（见 Releases）
```

### 使用 CLI

```bash
# 分析当前目录的代码库
codegraph analyze

# 搜索符号
codegraph query "UserService"

# 查看符号上下文
codegraph context "UserService"

# 影响分析
codegraph impact "UserService"

# 查看索引状态
codegraph status

# 启动 MCP 服务器
codegraph mcp

# 启动 HTTP API 服务
codegraph serve
```

### MCP 工具

CodeGraph 提供以下 MCP 工具：

| 工具 | 说明 |
|------|------|
| `list_repos` | 列出所有已索引的仓库 |
| `query` | 搜索代码符号 |
| `context` | 获取符号的上下文视图 |
| `impact` | 变更影响分析 |

## 开发

```bash
# 运行测试
cargo test --workspace

# 格式化代码
cargo fmt

# 代码检查
cargo clippy --workspace --all-targets -- -D warnings
```

## 许可证

本项目采用 MIT OR Apache-2.0 双许可证授权。
