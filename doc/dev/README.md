# pm-sysd-w 开发文档

pm-sysd-w (胖喵稳哒): Systemd style on Windows.

模仿 systemd 的风格和使用体验, 管理 Windows 上的进程 (service).

不兼容 systemd, 只实现 systemd 的一小 部分功能 (subset).

名称 "稳哒" 来自网络名句 "稳稳哒接住你" (狗头

## 编译目标平台

pm-sysd-w 支持 2 个编译目标:

| 目标    | 三元组                     | 用途           |
| ------- | -------------------------- | -------------- |
| Windows | `x86_64-pc-windows-msvc`   | 主要支持平台   |
| Linux   | `x86_64-unknown-linux-gnu` | 仅用于方便测试 |

Linux 目标仅用于方便测试: Linux 已有 systemd, pm-sysd-w 在 Linux 上几乎没有
实际用途. 在 Linux 上能测试的只是 pm-sysd-w 全部功能的一部分 (可能测不全);
Windows 上的测试才覆盖全部功能. 由于在 Windows 上测试较为麻烦, 开发过程中 先在
Linux 上快速验证, 之后再到 Windows 上做完整验证.

## 二进制程序

pm-sysd-w 编译出 3 个二进制程序:

| 程序           | 角色              | 说明                                                      |
| -------------- | ----------------- | --------------------------------------------------------- |
| `pm-sysd-init` | 类似 "PID 1"      | 常驻程序, 管理由自己启动的进程 (service), 基于 tokio 异步 |
| `pm-sysd-ctl`  | 类似 `systemctl`  | 主要命令行工具, 通过与 `pm-sysd-init` 交互完成操作, 同步  |
| `pm-sysd-log`  | 类似 `journalctl` | 日志查看工具, 通过与 `pm-sysd-init` 交互读取日志, 同步    |

3 个二进制程序 分别对应 3 个 cargo feature:

| 二进制         | feature    |
| -------------- | ---------- |
| `pm-sysd-init` | `cli_init` |
| `pm-sysd-ctl`  | `cli_ctl`  |
| `pm-sysd-log`  | `cli_log`  |

默认 feature 不包含这 3 个 feature. 正常情况下, 应分别启用对应的 feature 并分别
编译 (不应同时启用 2 个 cli feature):

```sh
cargo build --release --features cli_init
cargo build --release --features cli_ctl
cargo build --release --features cli_log
```

这样 `pm-sysd-init --version`, `pm-sysd-ctl --version` 与
`pm-sysd-log --version` 的输出会不同 (pm-bin 的 `--version` 会显示启用的所有
feature).

## 模块结构

| 模块        | 说明                                    |
| ----------- | --------------------------------------- |
| `cli`       | 命令行程序 公共模块 (仅 cli feature)    |
| `cli::init` | 实现 `pm-sysd-init` 程序 (CLI)          |
| `cli::ctl`  | 实现 `pm-sysd-ctl` 程序 (CLI)           |
| `cli::log`  | 实现 `pm-sysd-log` 程序 (CLI)           |
| `jsonl`     | init 与 ctl 之间的 JSONL 通信协议       |
| `conf`      | 调用 pm-tc 读取 toml 配置文件           |
| `db`        | sqlite 数据库 抽象/封装                 |
| `sysd`      | 服务管理 核心逻辑                       |
| `logd`      | 日志 核心逻辑 (类似 journald, 简化实现) |
| `pa`        | 底层平台 (操作系统) 抽象接口            |
| `p_w`       | Windows (主要平台) 的 pa 实现           |
| `p_u`       | Linux (仅测试) 的 pa 实现               |

## 配置文件 (依赖 pm-tc)

pm-sysd-w 不使用 systemd 那种 "一堆配置文件" 的方式, 而是采用 **集中式** 配置:
把很多 unit 的配置写在一个 toml 配置文件中, 由 pm-tc (胖喵模板配置工具) 读取.

pm-tc 已在 crates.io 发布 `0.1.0-a3` 版本, 源码见 `~/pm-tc/`.

加载配置后, pm-sysd-w 调用 pm-tc 的 `task::plan("sysd")`, 从返回的 `PlanDebug`
中拿到所有 unit 配置. 每个 unit 对应配置文件中的一个 `[[t.sysd]]` 任务实例, 通过
`env:XXX` 字段传递 unit 配置 (示例):

```toml
pm_tc = 1

[[t.sysd]]
"env:Unit" = "test-run.service"
"env:Unit.Description" = "Run a test program"

"env:Service.ExecStart" = "path/to/bin --arg"

"env:Service.Restart" = "always"
"env:Service.RestartSec" = "10"

"env:Install.WantedBy" = "default.target"

"env:Win.CreateNoWindow" = "yes"
```

其中 `env:Win.CreateNoWindow` 是 pm-sysd-w 专属扩展 (仅 Windows 有效, Linux
忽略).

bool 字段支持 systemd 语法: `1` / `yes` / `true` / `on` 表示 正值, `0` / `no` /
`false` / `off` 表示 负值.

## 通信 (IPC)

pm-sysd-init 与 pm-sysd-ctl / pm-sysd-log 的通信方式:

- Linux: 使用当前目录的 UNIX socket (仅用于方便测试, 权限默认, 无需专门处理).
- Windows: 使用命名管道 (Named pipe), 需要严格权限控制; 只允许本机程序连接, 禁止
  网络连接.

### 传输协议 (JSONL)

数据传输基于 JSONL (单行 JSON), 使用 `serde_json` 编解码.

ctl / log 连上 init 后, 首先进行版本号握手 (客户端发送, init 回复).
握手消息格式:

```json
{ "pm-sysd-w": 1, "data": { "version": "0.1.0-a1" } }
```

- `pm-sysd-w`: 传输协议格式版本 (信封结构版本), 当前固定为 `1`.
- `data.version`: pm-sysd-w 软件版本号 (字符串, semver), 用于版本检查.

握手流程:

1. 客户端 (ctl / log) 连接后, 发送上述握手消息 (携带自己的 `data.version`).
2. init 收到后, 回复同样结构的握手消息 (携带自己的 `data.version`).
3. 双方各自比较对方 `data.version` 是否与自己一致; 不一致则报错退出,
   不应继续操作.

只有握手消息携带版本号, 后续消息无需版本号.

传输协议属于内部 API, 不作为外部稳定 API 提供; 小版本更新也可能引入 BREAKING
change. 原则上 init 与 ctl / log 的版本应一致.

## pm-sysd-init

允许一个 Windows 系统上同时运行多个 pm-sysd-init 实例.
使用命名管道进行实例唯一性 检测 (同名实例只能有 1 个进程, 重复启动则报错退出).

命名管道名称前缀: `pm_sysd_w_`.

| 实例          | 启动参数        | 命名管道                              | 权限                   |
| ------------- | --------------- | ------------------------------------- | ---------------------- |
| 系统级        | `--system`      | `pm_sysd_w_system_default`            | 仅管理员               |
| 系统级 (只读) | `--system`      | `pm_sysd_w_system_default_ro`         | 本机普通用户可只读查询 |
| 用户级        | `--user`        | `pm_sysd_w_user_NAME` (NAME 为用户名) | 仅当前用户             |
| 自定义        | `--machine=XXX` | `pm_sysd_w_m_XXX`                     | 仅当前用户             |

- 系统级实例通常作为 Windows 服务运行, 开机自动启动; 用 `pm-sysd-ctl --system`
  操作.
- 用户级实例在每个登录用户登录后自动运行; 用 `pm-sysd-ctl --user` 操作.
- 自定义实例由用户手动启动; 用 `pm-sysd-ctl --machine=XXX` 操作.

pm-sysd-init 启动参数 (命令行):

- `--system`: 运行系统级实例.
- `--user`: 运行当前用户实例.
- `--machine=XXX`: 运行用户自定义名称实例.
- `--conf=PATH`: 指定主配置文件 (pm-tc toml) 路径 (必填).

`--system` / `--user` / `--machine=XXX` 启动时必选其一.

### 数据库

pm-sysd-init 使用 sqlite 数据库存储持久状态 (例如某个 service 是否 enable, 是否
mask 等). 数据库格式属于内部 API, 不作为外部稳定 API 提供.

默认数据库文件名为 `pm_sysd_w_v1.db`, 与主配置文件在同一目录: 主配置文件为
`dir/XXX.toml` 时, 数据库文件为 `dir/pm_sysd_w_v1.db`.

数据库文件路径可在主配置文件中修改, 例如:

```toml
[env]
SYSD_DB = "path/to/XXX.db"
```

`SYSD_DB` 为绝对路径; 若为相对路径, 则相对于主配置文件所在目录.

pm-sysd-init 只在 启动时 读取 `SYSD_DB` 并 打开数据库; 之后 即使 重新加载
配置文件 (`daemon-reload`), 也不会 更换数据库文件 (路径), 仍使用 当初打开的
数据库.

日志数据 (`logd` 模块) 也存入此 sqlite 数据库. 数据库 schema 详见
[db.md](db.md).

## pm-sysd-ctl

模仿 systemctl 的风格 (包括输出格式), 但不使用 pager; 只是相似, 不追求完全一致.
不要出现 `systemd` 等词, 因为本项目并不是 systemd.

(初版) 支持的命令 (最简实现方式):

```sh
list-units
list-timers
is-active
is-failed
status
show
cat
list-dependencies

start
stop
reload
restart
try-restart
reload-or-restart
try-reload-or-restart
kill
clean
reset-failed
whoami

enable
disable
reenable
preset
is-enabled
mask
unmask
get-default
set-default

show-environment
set-environment
unset-environment
import-environment

daemon-reload

soft-reboot  # 重启 pm-sysd-init 自身及所有被管理服务 (不是 Windows 重启)
exit  # 退出 pm-sysd-init
```

这些命令 CLI 都要有, 但初版不一定实际实现功能, 可以只输出 `TODO`; 也不必支持更多
参数/选项.

支持的选项 (含义基本与 systemd 一致或相似, 模仿风格):

```sh
-h --help

# --version 由 pm-bin 处理, pm-sysd-w 无需特殊处理
#--version

--system
--user

-M --machine=NAME  # 指定 pm-sysd-w 实例

--wait
--no-block

# 用于 `enable --now` 和 `disable --now`
--now
```

## pm-sysd-log

模仿 journalctl 的风格 (包括输出格式), 但不使用 pager; 只是相似, 不追求完全一致.

初版主要实现 `-eu UNIT` 的功能: 跳到日志末尾 (`-e`) 并按 unit 过滤 (`-u`),
输出最近若干条日志. 日志由 `logd` 模块写入 sqlite 数据库, 通过 init 读取.

支持的选项:

```sh
-h --help

-e  # 跳到日志末尾 (默认行为)

-u --unit=UNIT  # 只显示指定 unit 的日志

-x --catalog  # 显示解释文本 (初版无效果, 保留)

--system
--user

-M --machine=NAME  # 指定 pm-sysd-w 实例
```

## 支持的 target

| target           | 说明                                                              |
| ---------------- | ----------------------------------------------------------------- |
| `default.target` | pm-sysd-init 启动后的默认 target                                  |
| `timers.target`  | (保留) 用于 timer                                                 |
| `exit.target`    | pm-sysd-init 退出时 (与所有 service 等冲突, 从而终止所有 service) |

目前不支持用户自定义 target.

## 初版范围

- 只支持 service 一种 unit 类型; 不支持 timers 等各种其它 unit 类型.
- 不支持 unit 间依赖关系.
- 不支持自定义 target.

## 项目约定

- 源码注释 / 文档注释 一律使用中文 (可以含空格), 标识符 / 配置关键字为英文.
- 禁止 `unsafe` 代码 (`#![deny(unsafe_code)]`).
- 错误处理: 每个模块定义自己的错误枚举, 顶层错误类型用 `From` 汇总; 错误显示仅为
  尽力而为, 便于开发调试.
- 依赖 `pm-bin` 提供 CLI 脚手架 (与 pm-tc 一致).
- 源码中遗留 `TODO` 表示尚未完成, 属正常.

## 代码 (文档) 格式化

```sh
cargo fmt  # rust 代码
deno fmt  # 其余文件 (比如 markdown)
```

## 构建与测试

```sh
cargo build
cargo test
cargo doc
cargo build --release
```
