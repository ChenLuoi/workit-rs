# workit-rs

[English](README.md) | 简体中文

本仓库用于开发 Rust 命令行工具 `wk`，它让一个任务可以涉及多个 Git 项目。工作区会话负责选择项目和发起任务。用户随后在任务目录中开启新的 agent 或编辑器会话，每个参与项目都有独立的 Git worktree。

可以用 `cargo install workit-rs --locked` 或 `npm install -g workit-rs` 安装。两个渠道都提供 `wk` 命令，工作区操作需要 Git。

## 从源码开发

需要 Rust 1.94 或更高版本和 Git，可以选用 `just`。使用 `cargo run --locked -- <参数>` 或 `just wk <参数>` 运行本地构建。仓库将 `WK_WORKSPACE` 指向已忽略的 `.wk-dev/`，Cargo 产物在 `target/`，自动化测试使用独立的临时工作区。

可以用一个远端项目体验任务流程。

```sh
just wk init --yes --tools codex --skills wk-cli,wk-workspace-workflow
just wk add https://github.com/ChenLuoi/codex-ops.git --description "Codex 运维工具"
just wk start feature/operations codex-ops -D
just wk status feature/operations
```

尚未注册项目时，`wk start` 会先提示用 `wk add <url>` 注册项目。没有活跃任务时，`wk run` 会提示用 `wk start` 发起任务。

使用 `start` 打印的启动命令开启新的任务会话。用 `just wk --help` 查看全部命令，使用 `-w <目录>` 指定其他工作区。`just clean-dev` 会删除整个本地开发工作区，包括其中的 Git 数据。

## 修改入口

| 功能 | 源码 |
| --- | --- |
| 命令行和交互 | `src/main.rs`、`src/prompt.rs`、`src/tool.rs` |
| 工作区配置、参考快照和 Git 操作 | `src/workspace.rs`、`src/git.rs` |
| 任务生命周期、状态和诊断 | `src/tasks.rs`、`src/report.rs`、`src/report/` |
| 生成的工作区和任务文档 | `templates/workspace/`、`templates/task/`、`src/templates.rs` |
| 官方工作区 skills | `templates/workspace/.agents/skills/`、`src/templates/skills.rs` |

工作区文档由团队在初始化后自行维护。官方 **wk-cli** skill 解释命令接口，**wk-workspace-workflow** 说明如何选择项目、交接和关闭任务。修改 CLI 行为时应同步更新相应 skill 的源文件。Cargo 把它们编入两个发布渠道共用的二进制。升级后，在工作区运行 `wk doctor` 可检查已注册的 skills，再用 `wk skills sync [技能名]` 显式刷新。

任务模板使用创建时记录的项目名、职责和实际起始引用及提交。任务 README 和 AGENTS 使用同一份项目记录生成。`tests/cli.rs` 通过实际运行二进制来验证这些行为。

工作区把任务技能存放在可提交的 `task-skills/<id>/` 目录。`src/preset.rs` 管理 `wk.yaml` 中的预设，每个预设记录项目组合、可选的起始引用和技能。可用 `just wk preset add api-web` 建立预设，再用 `just wk start feature/login --preset api-web` 创建任务。加上 `--interactive` 可仅调整本次选择，也可以直接指定项目和 `--skills one,two`。`src/task_skills.rs` 为新任务会话保存所选技能的完整副本，并支持恢复中断的任务创建。

## 验证与发布

```sh
just check
just smoke-interactive
just smoke-npm
cargo package --locked --registry crates-io
```

推送和拉取请求的 CI 覆盖 Ubuntu、macOS 和 Windows。集成测试在临时目录创建真实 Git 仓库。在 Linux 和 macOS 上，`just smoke-interactive` 使用 Node.js 20+ 和系统的 `script` 命令，模拟初始化工作区、添加项目、启动任务、配置和选择启动工具时的键盘交互。`just smoke-npm` 将本机二进制与 npm 启动器分别打包，使用临时 npm 前缀离线验证安装、版本和工作空间初始化。

两个渠道的主包均为 `workit-rs`。crates.io 提供源码，npm 主包通过可选依赖选择预编译二进制。平台包使用 `@workit-rs/<平台>`，覆盖 Linux x64/arm64（glibc 和 musl）、macOS x64/arm64 和 Windows x64。

1. 在 GitHub 的 `release` environment 配置 `CARGO_REGISTRY_TOKEN` 和 `NPM_TOKEN`。首发前需验证 crates.io 账号邮箱，并授予令牌创建 `workit-rs` 的权限。npm 发布账号需要无 scope 主包和 `@workit-rs` 下七个平台包的公开发布权限。使用令牌的 CI 发布方式需要具有写权限且启用 Bypass 2FA 的 granular token。
2. 对齐 `Cargo.toml`、`Cargo.lock`、`package.json` 的版本及所有可选依赖版本。手动运行 Release workflow 可验证版本、构建矩阵和打包产物。
3. 在通过验证的提交上推送相应的标签（例如 `v0.1.0`）。发布会等待三个系统的 CI 以及七个目标的安装验证通过，其中 musl 产物在 Alpine 运行。npm 先发布平台包，最后发布主包。若发布作业失败，在同一次运行中重试失败的作业即可复用原始产物。
