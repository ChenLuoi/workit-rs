# workit-rs

[English](README.md) | 简体中文 · [仓库开发约定（英文）](AGENTS.md)

`wk` 是用于多仓库协同开发的 Rust 命令行工具。每个项目在 `repos/` 下克隆一次。每个任务有一个独立目录，里面并排放置参与项目的 Git worktree，可在这个目录启动 Codex、Claude Code 或其他交互程序。

## 本地开发

源码开发需要 Rust 1.94 或更高版本及 Git。以下快捷命令使用 `just`。

在源码仓库中，`just wk` 和 `cargo run --locked --` 默认使用 Git 忽略的 `.wk-dev/` 作为本地开发空间。首次运行 `just wk init` 后，配置、克隆的项目、任务和 worktree 都保存在这个目录。Cargo 构建产物位于 `target/`，自动化测试使用临时目录。

```sh
just wk init
just wk status
just wk add ../chen/codex-ops
just wk start test-cli codex-ops
just wk run test-cli -- codex
```

要操作其他空间，可以用 `just wk init /tmp/wk-demo` 初始化，再用 `just wk -w /tmp/wk-demo status` 查看。显式路径优先于本地 Cargo 配置提供的 `WK_WORKSPACE` 默认值。`add` 的相对路径仍从调用目录解析。通过源码运行时，`start` 打印的启动命令可直接使用当前构建的二进制。

## 安装与使用

版本发布后，可从任一渠道安装，命令名均为 `wk`。

```sh
npm install -g workit-rs
# 或
cargo install workit-rs --locked
```

```sh
wk init ~/work/cyder
cd ~/work/cyder
wk add git@github.com:cyder-hub/cyder-api.git
wk add git@github.com:ChenLuoi/codex-ops.git
wk start auth-refactor cyder-api codex-ops
```

最后一条命令创建 `worktrees/auth-refactor/{cyder-api,codex-ops}/`，在两个仓库中分别检出 `wk/auth-refactor` 分支，并打印可复制的启动命令。任选一个已安装的程序运行。

```sh
wk run auth-refactor -- codex
wk run auth-refactor -- claude
```

`run` 在任务根目录执行程序，透传参数、终端和退出码。退出程序后，可再次进入同一任务。每个项目在自己的目录中测试、提交和推送。

## 命令

| 命令 | 行为 |
| --- | --- |
| `wk init [dir]` | 建立工作空间，合并必要的忽略规则，保留已有的配置和协作说明。 |
| `wk add <url> [--name alias] [--base origin/develop]` | 克隆并登记项目。默认从 URL 推导项目名，开发基线取自 origin/HEAD。支持 SSH、HTTPS 和现有的本地仓库路径。 |
| `wk sync [project...]` | 按配置补齐 clone，对已有项目执行 git fetch origin。 |
| `wk start <task> <project...> [--no-fetch]` | 默认先 fetch，再按各仓库的基线提交建立任务。离线工作可使用 --no-fetch。 |
| `wk run <task> -- <program> [args...]` | 在任务根目录启动程序。 |
| `wk path <task> [project]` | 输出任务或指定项目 worktree 的绝对路径，适合 cd 或编辑器。 |
| `wk status [task] [--json] [--fetch]` | 列出项目、任务阶段、分支和文件改动。远程比较使用本地缓存，--fetch 主动刷新。 |
| `wk doctor` | 只读检查 Git、配置、项目来源和任务工作区，发现异常时返回非零退出码。 |
| `wk close <task>` | 先检查所有成员，再移除干净的 worktree，保留任务分支、任务根目录中的笔记和归档记录。 |

在工作空间的子目录里运行 `wk` 会向上寻找最近的 `wk.yaml`。也可在外部使用 `wk --workspace <dir> status`。

## 配置与恢复

共享配置 `wk.yaml` 可以随工作空间仓库提交。克隆工作空间后，运行 `wk init` 和 `wk sync` 即可建立本地目录并拉取项目。

```yaml
version: 1
projects:
  cyder-api:
    url: git@github.com:cyder-hub/cyder-api.git
  codex-ops:
    url: git@github.com:ChenLuoi/codex-ops.git
    base: origin/develop
```

`wk add` 写回清单时会重新序列化 YAML，手写注释和排版可能丢失。

共享任务说明可写入 `tasks/<task>.md`。每个任务目录中的 `AGENTS.md` 列出项目并指向该说明。本机的 `.wk/tasks/<task>.json` 记录项目、基线、起始提交和任务阶段。工作空间的写命令使用文件锁，配置和任务记录采用临时文件原子替换。

如果创建中断，使用相同的任务名和项目集合重跑 `wk start`，它会从记录的提交补齐缺失的 worktree。若关闭中断，解决错误后重跑 `wk close`。关闭前需要处理项目内的修改、未跟踪文件及被忽略的文件（例如 `.env`）。已有任务分支和同名归档记录会占用任务名。原型使用普通 Git clone 和 UTF-8 路径。

## 验证与发布

```sh
just check
just smoke-npm
cargo package --locked --registry crates-io
```

集成测试在临时目录创建真实 Git 仓库。`just smoke-npm` 将本机二进制与 npm 启动器分别打包，使用临时 npm 前缀离线验证安装、版本和工作空间初始化。

两个渠道的主包均为 `workit-rs`。crates.io 提供源码，npm 主包通过可选依赖选择预编译二进制。平台包使用 `@workit-rs/<平台>`，覆盖 Linux x64/arm64（glibc 和 musl）、macOS x64/arm64 和 Windows x64。

1. 在 GitHub 的 `release` environment 配置 `CARGO_REGISTRY_TOKEN` 和 `NPM_TOKEN`。npm 发布账号需要主包和 `@workit-rs` scope 下各平台包的公开发布权限。
2. 对齐 `Cargo.toml`、`Cargo.lock`、`package.json` 的版本及所有可选依赖版本。手动运行 Release workflow 可验证版本、构建矩阵和打包产物。
3. 推送相应的版本标签（例如 `v0.1.0`）触发双渠道发布。所有平台构建成功后才会发布。npm 先发布平台包，最后发布主包。若某个发布作业失败，可在同一次运行中重试失败的作业。
