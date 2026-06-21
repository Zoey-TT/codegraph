# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-06-18

### Added

- 多语言 AST 解析：支持 Rust、TypeScript/JavaScript、Python、Go、Java、C/C++、C#、Kotlin、Swift、PHP、Ruby、Dart、Vue、COBOL、Markdown
- 12 阶段解析管线（Pipeline）编排
- 内存知识图谱模型与 JSONL 持久化
- 混合搜索引擎：Tantivy 全文检索 + 向量语义搜索 + RRF 融合
- 增量索引：基于 Git diff 和内容哈希
- 社区检测与执行流追踪
- MCP 服务器与核心工具（list_repos、query、context、impact）
- HTTP API 服务（Axum）
- CLI 命令行工具（analyze、setup、mcp、serve、list、status、clean、query、context、impact、communities）

### Notes

- `wiki` 命令已隐藏，尚未实现
- Cypher 查询当前版本不支持
