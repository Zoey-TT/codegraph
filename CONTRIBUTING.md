# Contributing to CodeGraph

感谢你对 CodeGraph 的兴趣！我们欢迎 issue、PR 和各种形式的贡献。

## 开发环境

- [Rust](https://rustup.rs/) 1.81.0 或更高版本

```bash
git clone <repository-url>
cd codegraph

# 默认构建
cargo build
```

## 代码风格

提交前请确保：

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --workspace
```

## 提交 PR

1. Fork 仓库并创建分支
2. 尽量保持提交历史清晰
3. 新增功能请补充测试
4. 确保 CI 检查通过
5. 在 PR 描述中说明改动原因和影响范围

## 测试

- 单元测试：`cargo test --workspace`
- 集成测试：`cargo test --test cli --test server --test registry`
- 基准测试：`cargo bench`（需要 `criterion`）

## Issue 报告

报告 bug 时，请提供：

- 复现步骤
- 期望行为 vs 实际行为
- Rust 版本：`rustc --version`
- 相关日志或错误输出

## 许可证

提交代码即表示你同意你的贡献将采用与项目相同的许可证：
`MIT OR Apache-2.0`。
