# pm-sysd-w 用户文档

pm-sysd-w (胖喵稳哒) 在 Windows 上以类似 systemd 的方式管理进程 (process).

使用 `pm-sysd-ctl` 操作 service, 用法类似 `systemctl`. 使用 `pm-sysd-log`
查看日志, 类似 `journalctl`.

pm-sysd-w 不兼容 systemd, 只实现其中一小部分功能 (subset). 本文档重点说明与
systemd 的 **差异**. 与 systemd 一致 (或高度相似) 的部分从简或省略.

## 概念

`service` 是 pm-sysd-w 的一种 unit 类型, 概念类似 systemd 的 service. 注意它不是
Windows 的 "Windows 服务", 实际是 `pm-sysd-init` 启动并管理的一个 (或一组) 进程.

`pm-sysd-init` 是常驻的守护进程 (类似 systemd 的 PID 1 角色), 负责启动和管理
service. `pm-sysd-ctl` / `pm-sysd-log` 与其 (实例) 通信, 完成操作.

## 与 systemd 的主要差异

- 配置文件: 使用 pm-tc 的 **集中式** toml 格式, 多个 unit 写在一个文件里, 而不是
  systemd 那样的多个 unit 文件.

- 部署: 默认不包含开机自启 / 登录后自启. 项目提供 `install/setup.ps1` 作为实验性
  安装方式 (见 `安装` 一节), 其它自动启动方式仍需要用户手动处理.

- 默认实例: `pm-sysd-ctl` / `pm-sysd-log` 未指定实例时, 优先尝试 `--system`,
  若系统实例未运行则回退到 `--user`. 这与 systemd 不同 (systemd
  默认只连接系统实例, 不会回退).

- 权限: 连接 `--system` 的普通用户 (非管理员) 自动改用只读连接, 只能进行只读查询
  (如查看状态 / 日志).

- `cat` 命令输出 JSON, 而不是 systemd 的 unit 文件内容.

- `soft-reboot` 是重启 `pm-sysd-init` 自身, 及所有被管理的 service. 不是重启
  Windows.

- Windows 关机 / 注销时无法保证优雅停止 service (详见下文).

- 日志保存在本地数据库中, 默认保留最近 72 小时. 另有 pm-sysd-w 专属命令
  `sqlite-vacuum` 用于清理/优化数据库.

## 安装

目前需要自行编译. 需要 Rust 工具链 (edition 2024), 仅支持 Windows 10 或更新版本.

```sh
cargo build --release --features cli_init
cargo build --release --features cli_ctl
cargo build --release --features cli_log
```

编译产物位于 `target/release/` 目录: `pm-sysd-init.exe`, `pm-sysd-ctl.exe`,
`pm-sysd-log.exe`.

### 安装脚本 (实验性)

项目提供 `install/setup.ps1`, 在项目根目录用 PowerShell 运行. 目前支持
`install-user` 与 `uninstall-user` 两个命令, 需要管理员权限.

```powershell
# 在项目根目录运行
powershell -ExecutionPolicy Bypass -File install\setup.ps1 install-user
powershell -ExecutionPolicy Bypass -File install\setup.ps1 uninstall-user
```

`install-user` 会:

1. 把 `pm-sysd-init.exe` / `pm-sysd-ctl.exe` / `pm-sysd-log.exe` 复制到
   `%LOCALAPPDATA%\pm-sysd-w\bin\`.

2. 把此目录加入当前用户 `PATH`.

3. 在 `%LOCALAPPDATA%\pm-sysd-w\pm_sysd_w.conf.toml` 写入空配置文件
   (已存在则不覆盖).

4. 把 `pm-sysd-init.exe --user` 安装为 Windows per-user 服务
   (用户登录时自动启动).

`uninstall-user` 只删除该 per-user 服务, 保留 `%LOCALAPPDATA%\pm-sysd-w\` 目录.

> **实验性支持**: 目前仅支持这两个命令. `pm-sysd-init` 现已实现 Windows
> 服务接口, per-user 服务会随用户登录自动启动. 其它自动启动方式 (任务计划程序 /
> 启动文件夹 / Run 注册表项) 仍由用户手动处理.

## 快速开始

先启动一个实例 (以当前用户实例 `--user` 为例), 加载集中式配置文件:

```sh
pm-sysd-init --user --conf=pm_sysd_w.conf.toml
```

再用 `pm-sysd-ctl` 操作 (指定同一实例), 比如:

```sh
pm-sysd-ctl --user start test-run

pm-sysd-ctl --user status test-run

pm-sysd-ctl --user enable test-run
```

查看日志, 退出实例:

```sh
pm-sysd-log --user -eu test-run.service

pm-sysd-ctl --user exit
```

## 实例 (instance)

一个系统上可同时运行多个 `pm-sysd-init` 实例, 用 `pm-sysd-ctl`
的实例选项选择操作目标:

| 选项             | 实例       | 说明                                                  |
| ---------------- | ---------- | ----------------------------------------------------- |
| `--system`       | 系统级实例 | 用于整台机器, 需管理员权限                            |
| `--user`         | 用户级实例 | 当前登录用户                                          |
| `--machine=NAME` | 自定义实例 | 手动启动, NAME 的字符仅限 [0-9A-Za-z_-], 长度 1..=100 |

- `pm-sysd-init` 启动时必须用 `--conf=PATH` 指定主配置文件, 并使用实例选项:
  `--system` / `--user` / `--machine=NAME` 不能同时使用, 只能指定一个.

- 默认不包含开机自启 / 登录后自启. 上述启动方式需要用户手动处理 (例如任务计划
  程序 / 启动文件夹 / Run 注册表项). 实验性的 `install/setup.ps1` 见 `安装`
  一节.

- `pm-sysd-ctl` / `pm-sysd-log` 未指定实例选项时, 优先尝试 `--system`.
  若系统实例未运行, 则自动回退到 `--user`. 连接 `--system` 时, 普通用户
  (非管理员) 自动改用只读连接.

## 配置文件

pm-sysd-w 使用 pm-tc 的集中式 toml 配置文件, 多个 unit 写在一个文件里.
格式版本固定为 `pm_tc = 1`. 完整字段参考见 [config.md](config.md).

示例 (见 `example/pm_sysd_w.conf.toml`):

```toml
pm_tc = 1

[[t.sysd]]
"env:Unit" = "test-run.service"
"env:Unit.Description" = "Run a test program"

"env:Service.ExecStart" = "path/to/bin --arg"

"env:Service.Restart" = "always"
"env:Service.RestartSec" = "10"

"env:Install.WantedBy" = "default.target"
```

- 每个 `[[t.sysd]]` 定义一个 unit (service).

- `env:Unit`: unit 名称.

- `env:Unit.Description`: 描述.

- `env:Service.ExecStart`: 启动命令.

- `env:Service.Restart` / `env:Service.RestartSec`: 重启策略.

- `env:Install.WantedBy`: 所属 target, 初版只有值为 `default.target` 的 unit
  才能 `enable`.

- `env:Win.CreateNoWindow`: (可选, Windows 专属) 启动时设置 `CREATE_NO_WINDOW`,
  避免 console 程序显示黑窗口. Linux 上忽略. 默认 `no`.

## pm-sysd-ctl 命令

(初版) 支持的命令, 风格与 `systemctl` 相似 (部分命令初版可能只输出 `TODO`):

```sh
list-units
list-timers

status
is-active
is-failed
is-enabled
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
reset-failed
clean

enable
disable
reenable
mask
unmask

daemon-reload
get-default
set-default
preset

show-environment
set-environment
unset-environment
import-environment

whoami

soft-reboot  # 重启 pm-sysd-init 自身及所有被管理 service
exit  # 退出 pm-sysd-init

sqlite-vacuum  # 清理/优化 sqlite 数据库
```

通用选项:

```sh
   --version
-h --help

   --system
   --user
-M --machine=NAME

   --wait
   --no-block
```

命令行为说明:

- `enable` / `disable` 支持 `--now` (例如 `enable --now`).

- `preset` 初版等效于 `disable`.

- unit 名称未写后缀时, 自动补 `.service` (例如 `status test` 等价于
  `status test.service`). `pm-sysd-log` 的 `-u` / `--unit` 同理.

- `mask` 行为与 systemd 一致: 支持 `--now` (默认只 mask, 加 `--now` 才停止). 被
  mask 的 unit 视为不存在, 但仍运行中的 service 会以 `masked` 状态显示, 且
  `start` 等命令会拒绝启动.

- `show` 与 `cat` 输出相同的全部受支持配置字段: `show` 为 `key=value` (key
  与配置一致, 如 `Service.ExecStart`), `cat` 为 pretty JSON (不模仿 systemd 的
  unit 文件格式).

- `list-units` / `status` / `list-timers` 的输出格式尽量模仿 `systemctl`,
  与本项目无关的字段省略.

- `list-units` 按 unit 名称排序. `daemon-reload` 后已从新配置移除但仍运行的 unit
  也会列出 (状态来自运行中的旧配置).

- `is-active`, `is-failed`, `is-enabled` 退出码与 systemd 一致.

- `set-default` 初版仅支持 `default.target`, 其它值报错.

- `daemon-reload` 重新读取配置文件并刷新 enable/mask 状态. reload 期间执行
  `enable` / `disable` / `reenable` / `mask` / `unmask` 会报错, 且不允许并发
  执行多个 `daemon-reload`.

- `list-dependencies` 因 unit 间依赖功能尚未实现, 返回空结果.

## pm-sysd-log

`pm-sysd-log` 查看日志 (类似 `journalctl`), 用法:

```sh
# 显示最近日志 (跳到末尾, 默认)
pm-sysd-log --user

# 只显示指定 unit 的最近日志 (-eu 组合)
pm-sysd-log --user -eu test-run.service
```

通用选项:

```sh
-e  # 跳到日志末尾 (默认)
-u --unit=UNIT
-x --catalog  # 初版无效果

   --system
   --user
-M --machine=NAME

-h --help
   --version
```

### 日志说明

- 日志仅保留最近 72 小时, pm-sysd-init 每 1 小时自动清理一次.

- service 进程的 stdout / stderr 会按行写入对应 unit 的日志.
  单行超长时会被拆分为多条日志. 输出不保证 UTF-8, 解码失败时会换成一条 error
  日志.

## 退出与重启实例

- `pm-sysd-ctl ... exit` 正常退出 pm-sysd-init.

- `pm-sysd-ctl ... soft-reboot` 重启 pm-sysd-init 自身, 及所有被管理 service
  (不是 Windows 重启).

- 外部信号: Linux 上 `SIGTERM`, Windows 上 `Ctrl+C`, 也会触发优雅退出.

## Windows 关机 / 注销时的限制

由于 Windows 的限制, 在 Windows 关机或用户注销时, pm-sysd-w 无法保证
(实际上几乎无法实现) 优雅停止 service:

- 在关机 / 注销阶段, Windows 不允许启动新进程 / 执行新程序, 所以 `ExecStop`
  很可能无法运行.

- 新版 Windows (Windows 10 / 11) 无法阻止关机, 系统会强制关机.
  而且每个程序只有很短的固定硬超时上限 (比如 5 秒). 也就是说, Windows
  最多等待每个程序 5 秒, 然后就会强制关机. 这个行为无法配置, 程序也无法修改.
