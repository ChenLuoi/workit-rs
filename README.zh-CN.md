# workit-rs

[English](README.md) | 简体中文

本仓库用于开发 Rust 命令行工具 `wk`，它让一个任务可以涉及多个 Git 项目。工作区会话负责选择项目和发起任务。用户随后在任务目录中开启新的 agent 或编辑器会话，每个参与项目都有独立的 Git worktree。

## 安装

从 [GitHub Releases](https://github.com/ChenLuoi/workit-rs/releases/latest) 下载 `wk-<平台>-v<版本>`。按系统选择 `linux-{x64,arm64}-{gnu,musl}`、`darwin-{x64,arm64}` 或 `win32-x64-msvc`，Windows 文件以 `.exe` 结尾。使用工作区功能需要能从 `PATH` 调用 Git。

首次用 mise 安装时，运行 `mise use -g github:ChenLuoi/workit-rs --minimum-release-age 0h` 即可得到 `wk`。升级已有安装时，请将命令中的工具名写为 `github:ChenLuoi/workit-rs@latest`。

以下命令适用于在使用 glibc 的 Linux x64 上直接安装 v0.1.3。

```sh
(
  set -e
  tmp=$(mktemp -d)
  trap 'rm -rf -- "$tmp"' EXIT
  cd "$tmp"
  version=0.1.3
  asset="wk-linux-x64-gnu-v$version"
  base="https://github.com/ChenLuoi/workit-rs/releases/download/v$version"
  curl -fsSLO "$base/$asset"
  curl -fsSLO "$base/SHA256SUMS"
  awk -v file="$asset" '$2 == file' SHA256SUMS | sha256sum -c -
  mkdir -p "$HOME/.local/bin"
  install -m 755 "$asset" "$HOME/.local/bin/wk"
  "$HOME/.local/bin/wk" --version
)
```

将 `~/.local/bin` 加入 `PATH` 后即可运行 `wk`。macOS 请选择对应的 Darwin 文件，校验命令使用 `shasum -a 256 -c -`。Windows 可用 `Get-FileHash <下载文件> -Algorithm SHA256` 与 `SHA256SUMS` 对照，随后将文件重命名为 `wk.exe`，放入 `PATH` 中的目录。

也可以用 `cargo install workit-rs --locked` 或 `npm install -g workit-rs` 安装。

## 加入共享工作区

克隆团队的工作区仓库，进入目录后运行 `wk setup`。第一步选择「个人配置」或「共享配置」，也可以用 `wk setup --local` 或 `wk setup --shared` 直接进入对应流程。

- 个人配置将项目和工具选择保存在 Git 忽略的 `.wk/local.yaml`。可以跟随团队全部项目，也可以只选自己使用的项目。例如，`wk setup --local api web` 只准备这两个项目。拉取团队更新后运行 `wk sync`。命令会列出失败的项目并以非零状态退出，已成功的项目可立即使用。
- 共享配置在 `wk.yaml` 中维护团队启用的工具、已注册的工作区 skills 和项目描述。例如，`wk setup --shared --tools codex,claude` 为团队启用这两个工具，个人配置可再选其中一部分。

## 清理损坏任务和查找历史

即使已经手动删除任务目录，仍可以运行 `wk close <任务名>`。遇到任务根仓库损坏或 worktree 缺失时，wk 会说明原因并请求确认。脚本可使用 `wk close <任务名> --force` 确认恢复清理。确认后会永久删除任务目录中的剩余文件，并将损坏原因写入历史记录的 `damage` 字段。完好的 worktree 仍需通过清洁度和锁定检查。

已关闭的任务保存在 `.wk/history/`。文件名以本地开始时间开头，精确到秒，例如 `2026—11—26—18—42—07—feature%2Flogin.yaml`。`started_at` 和 `closed_at` 也使用这个日期格式。同一秒内的同名任务用 `—2` 等序号区分。

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

`wk start` 会在 `tasks/<task>/.git` 创建一个空的裸仓库，让 Git 和 Codex 将任务目录识别为根目录。请在各项目的 worktree 中查看和提交代码更改。任务根目录下的共享指南、技能和笔记不受 Git 跟踪，`wk close` 会将它们一并删除。

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

创建、重新打开或更新 PR 时，CI 会在 Ubuntu、macOS 和 Windows 上运行。合并 PR 或直接推送到 `master` 时，CI 会在 `master` 分支上检查此次推送的提交。推送版本标签时，由 Release 工作流调用同一套检查。各平台的集成测试会在临时目录中创建真实 Git 仓库，并构建供冒烟测试复用的 release 二进制。

在 Linux 和 macOS 上，`just smoke-interactive` 使用 Node.js 20+ 和 `socat`，模拟初始化工作区、添加项目、启动任务、配置和选择启动工具时的键盘交互。`just smoke-npm` 将本机二进制与 npm 启动器分别打包，使用临时 npm 前缀离线验证安装、版本和工作空间初始化。

两个渠道的主包均为 `workit-rs`。crates.io 提供源码，npm 主包通过可选依赖选择预编译二进制。平台包使用 `@workit-rs/<平台>`，覆盖 Linux x64/arm64（glibc 和 musl）、macOS x64/arm64 和 Windows x64。

1. 在 GitHub 的 `release` environment 配置 `CARGO_REGISTRY_TOKEN` 和 `NPM_TOKEN`。首发前需验证 crates.io 账号邮箱，并授予令牌创建 `workit-rs` 的权限。npm 发布账号需要无 scope 主包和 `@workit-rs` 下七个平台包的公开发布权限。使用令牌的 CI 发布方式需要具有写权限且启用 Bypass 2FA 的 granular token。
2. 对齐 `Cargo.toml`、`Cargo.lock`、`package.json` 的版本及所有可选依赖版本。手动运行 Release workflow 可验证版本、构建矩阵和打包产物。
3. 在通过验证的提交上推送相应的标签（例如 `v0.1.0`）。发布会等待三个系统的 CI 以及七个目标的安装验证通过，其中 musl 产物在 Alpine 运行。npm 先发布平台包，最后发布主包。若发布作业失败，在同一次运行中重试失败的作业即可复用原始产物。

演练还会对全部八个 npm 归档执行发布脚本的 dry-run。如果需要在后续提交中修正发布脚本，可在默认分支手动运行 Release，并将 `npm_run_id` 设为原标签运行的 ID。补发流程验证标签提交和构建结果，再发布原始 npm 归档。默认分支当前的包版本须与标签一致。
