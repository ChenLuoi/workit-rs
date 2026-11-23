# workit-rs

[English](README.md) | 简体中文 · [仓库开发约定（英文）](AGENTS.md)

`wk` 是用于多仓库协同开发的 Rust 命令行工具。每个项目在 `repos/` 下克隆一次。每个任务有一个独立目录，里面并排放置参与项目的 Git worktree，可在这个目录启动 Codex、Claude Code 或其他交互程序。

`repos/<项目>/` 是可以浏览源码的参考副本。开发先执行 `wk start <任务> <项目...>`，在 `tasks/<任务>/<项目>/` 中编辑、构建和提交。任务提交共享 Git 数据，参考副本中检出的文件保持原样。

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
wk start auth-refactor
```

在项目列表中用 ↑/↓ 移动，空格切换选中，回车确认。本例勾选两个项目。再按回车使用各项目配置的 `base`，或输入 `n` 后回车，逐个按前缀筛选 `origin` 分支。全部选完后，`wk` 创建 `tasks/auth-refactor/{cyder-api,codex-ops}/`，分别检出新的 `wk/auth-refactor` 分支，并打印可复制的启动命令。任选一个已安装的程序运行。

```sh
wk run auth-refactor -- codex
wk run auth-refactor -- claude
wk run auth-refactor -- code .
```

这三条命令都打开 `tasks/auth-refactor/`。VS Code 的同一个窗口里能看到两个仓库和公共文件，Codex 与 Claude Code 也能访问同一批文件。使用其他编辑器时，可运行 `wk path auth-refactor` 取得这个目录。测试、提交和推送仍在各项目自己的 worktree 中执行。

## 命令

| 命令 | 行为 |
| --- | --- |
| `wk init [dir]` | 建立工作空间，合并必要的忽略规则，保留已有的配置和协作说明。 |
| `wk add <url> [directory] [--base <ref>]`（别名：`wk clone`） | 将 SSH、HTTPS 或本地仓库克隆到新目录 `repos/<directory>/` 并登记，目录名即项目名。 |
| `wk sync [project...]` | 按配置补齐 clone，对已有项目执行 git fetch origin。 |
| `wk remove <project>`（别名：`wk rm`） | 从清单移除项目并删除干净的本地 clone。若 clone 已缺失，则移除登记。 |
| `wk start <task> [project...] [-D] [--no-fetch]` | 交互多选项目，也可直接列出项目名。`--default` / `-D` 跳过基线确认。`--no-fetch` 使用本地缓存，分支选择也可离线完成。 |
| `wk run <task> -- <program> [args...]` | 在任务根目录启动程序。 |
| `wk path <task> [project]` | 输出任务或指定项目 worktree 的绝对路径，适合 cd 或编辑器。 |
| `wk status [task] [--json] [--fetch]` | 列出项目、任务阶段、分支和文件改动。远程比较使用本地缓存，--fetch 主动刷新。 |
| `wk doctor` | 只读检查 Git、配置、项目来源和任务工作区，发现异常时返回非零退出码。 |
| `wk close <task>` | 检查所有成员 worktree，通过 Git 移除后删除整个任务目录及其中的共享资源。保留项目分支和任务归档记录。 |

在工作空间的子目录里运行 `wk` 会向上寻找最近的 `wk.yaml`。也可在外部使用 `wk --workspace <dir> status`。

脚本中新建任务时需显式传入至少一个项目，例如 `wk start auth-refactor cyder-api codex-ops -D`。非交互执行直接采用各项目配置的默认基线。交互选择的分支仅对本次任务生效。在提示中按 Esc 退出不会创建任务记录、分支或 worktree。

在交互终端中只给 URL 时，`wk clone` 和 `wk add` 会先询问目录名，再列出远端分支并选中默认分支。连续回车两次即可接受默认值。要选择其他基线，输入短分支名的前缀（区分大小写），例如输入 `rel` 可匹配 `release/2.x`。用 ↑/↓ 选择、回车确认，清单会记录远程跟踪引用，如 `origin/release/2.x`。按 Esc 可在克隆前取消。

脚本调用直接采用默认值。显式给出目录名或 `--base` 中的任一项，也会跳过两个提示。指定 tag、提交，或远端未公布默认分支时，可使用 `--base <ref>`。

查询远端分支、克隆项目或从 `origin` 拉取时，终端会显示带操作名称的等待动画。动画写入 stderr，重定向 stderr 时自动关闭。

```sh
wk clone git@github.com:cyder-hub/cyder-api.git api-preview --base origin/develop
```

目录参数是 `repos/` 下的一级目录名。名称已登记或目标已存在（包括空目录、来源匹配的 clone）都会报错。同一 URL 可以用不同名称再次添加。已登记的项目缺少本地 clone 时，使用 `wk sync <项目名>` 恢复。显式指定的基线由 `wk start` 解析为提交。

参考副本中有暂存、修改、未跟踪或被忽略的文件时，`wk status` 会显示原因，`wk doctor` 会报错。手动切离已知默认分支、参考副本的 HEAD 含仅本地可达的提交，也会被检测。涉及该项目副本或任务的操作会停下，直到你妥善保存改动并恢复参考副本。这是 `wk` 命令的检查，普通 clone 仍可通过 Git 和编辑器直接访问。

清除项目时，先关闭关联任务，再执行 `wk rm <项目名>`。未关闭或中断的任务、额外登记的 worktree（包括手动创建的）都会阻止移除。即使父仓库显示干净，只要子模块目录非空，或 `.git/modules` 中存有子模块仓库，`wk rm` 也会拒绝删除。删除 clone 前，`wk` 会检查本地引用和 reflog 中的提交是否已被本地已知的 `origin` 分支包含。即使任务已关闭，保留的任务分支含未推送提交也会拒绝删除。请先推送要保留的工作。已关闭的任务记录和远端仓库会保留。

## 配置与恢复

共享配置 `wk.yaml` 可以随工作空间仓库提交。克隆工作空间后，运行 `wk init` 和 `wk sync` 即可建立本地目录并拉取项目。

```yaml
version: 1
projects:
  cyder-api:
    name: cyder-api
    url: git@github.com:cyder-hub/cyder-api.git
    base: origin/main
  codex-ops:
    name: codex-ops
    url: git@github.com:ChenLuoi/codex-ops.git
    base: origin/develop
```

日常使用 CLI 维护 `wk.yaml`。每个项目完整记录名称、URL 和基线，包括采用的默认值。确需手动调整时，只修改个别字段值，并保持项目键与 `name` 一致。请避免改动清单结构或添加注释，CLI 写回时会重新序列化，注释和自定义排版可能丢失。

任务资源和各项目的 worktree 并排存放。例如，放入任务说明和测试数据后，目录可以是这样：

```text
tasks/auth-refactor/
├── AGENTS.md       # wk start 生成
├── brief.md
├── fixtures/
├── cyder-api/      # Git worktree
└── codex-ops/      # Git worktree
```

工作空间仓库忽略整个 `tasks/`。**关闭任务也会删除任务目录里的说明、测试数据、编辑器配置和其他本地文件。** 需要长期保留的内容，请先提交到对应项目，或复制到任务目录以外。本机的 `.wk/tasks/<task>.json` 记录参与项目、基线、起始提交和任务阶段。工作空间的写命令使用文件锁，并原子替换配置和任务记录。

任务记录写入后，重跑 `wk start <任务名>` 会按已记录的项目和提交恢复。若重试时显式列出项目，集合须与记录一致。若关闭中断，解决错误后重跑 `wk close`。关闭前需要处理项目内的修改、未跟踪文件及被忽略的文件（例如 `.env`）。已有任务分支和同名归档记录会占用任务名。原型使用普通 Git clone 和 UTF-8 路径。

## 验证与发布

```sh
just check
just smoke-interactive
just smoke-npm
cargo package --locked --registry crates-io
```

集成测试在临时目录创建真实 Git 仓库。在 Linux 和 macOS 上，`just smoke-interactive` 使用 Node.js 20+ 和系统的 `script` 命令，模拟添加项目和启动任务时的键盘交互。`just smoke-npm` 将本机二进制与 npm 启动器分别打包，使用临时 npm 前缀离线验证安装、版本和工作空间初始化。

两个渠道的主包均为 `workit-rs`。crates.io 提供源码，npm 主包通过可选依赖选择预编译二进制。平台包使用 `@workit-rs/<平台>`，覆盖 Linux x64/arm64（glibc 和 musl）、macOS x64/arm64 和 Windows x64。

1. 在 GitHub 的 `release` environment 配置 `CARGO_REGISTRY_TOKEN` 和 `NPM_TOKEN`。npm 发布账号需要主包和 `@workit-rs` scope 下各平台包的公开发布权限。
2. 对齐 `Cargo.toml`、`Cargo.lock`、`package.json` 的版本及所有可选依赖版本。手动运行 Release workflow 可验证版本、构建矩阵和打包产物。
3. 推送相应的版本标签（例如 `v0.1.0`）触发双渠道发布。所有平台构建成功后才会发布。npm 先发布平台包，最后发布主包。若某个发布作业失败，可在同一次运行中重试失败的作业。
