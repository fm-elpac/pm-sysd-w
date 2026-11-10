# pm-sysd-w 用户文档

pm-sysd-w (胖喵稳哒): Systemd style on Windows.

使用 `pm-sysd-ctl` 管理 Windows 上的进程 (service), 用法类似 `systemctl`.

## 安装

需要 Rust 工具链 (edition 2024).

```sh
cargo build --release --features cli_init
cargo build --release --features cli_ctl
cargo build --release --features cli_log
# 二进制位于:
# target/release/pm-sysd-init
# target/release/pm-sysd-ctl
# target/release/pm-sysd-log
```

## 快速开始

先启动一个实例 (以自定义实例 `dev` 为例), 加载集中式配置文件:

```sh
pm-sysd-init --machine=dev --conf=pm_sysd_w.conf.toml
```

再用 `pm-sysd-ctl` 操作 (指定同一实例):

```sh
pm-sysd-ctl --machine=dev daemon-reload

pm-sysd-ctl --machine=dev start test-run.service

pm-sysd-ctl --machine=dev status test-run

pm-sysd-ctl --machine=dev enable test-run.service
```

## 实例 (instance)

一个系统上可同时运行多个 pm-sysd-init 实例, 用 `pm-sysd-ctl` 的实例选项选择要
操作的对象:

| 选项             | 实例       | 说明                                    |
| ---------------- | ---------- | --------------------------------------- |
| `--system`       | 系统级实例 | 通常作为 Windows 服务开机自启, 需管理员 |
| `--user`         | 用户级实例 | 当前用户登录后自动运行                  |
| `--machine=NAME` | 自定义实例 | 手动启动, 名称自定义                    |

## 配置文件

pm-sysd-w 使用 pm-tc 的集中式 toml 配置文件, 多个 unit 写在一个文件里. 格式版本
固定为 `pm_tc = 1`. 完整字段参考见 [config.md](config.md).

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
- `env:Install.WantedBy`: 所属 target (初版仅支持 `default.target` 等内置
  target).
- `env:Win.CreateNoWindow`: (可选, Windows 专属) 启动时设置 CREATE_NO_WINDOW,
  避免 console 程序显示黑窗口; Linux 上忽略. 默认 `no`.

## pm-sysd-ctl 命令

(初版) 支持的命令, 风格与 `systemctl` 相似 (部分命令初版可能只输出 `TODO`):

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
soft-reboot
exit
```

`enable` / `disable` 支持 `--now` (例如 `enable --now`).

通用选项:

```sh
-h --help
--system
--user
-M --machine=NAME
--wait
--no-block
```

## pm-sysd-log

`pm-sysd-log` 查看日志 (类似 `journalctl`), 用法:

```sh
# 显示最近日志 (跳到末尾, 默认)
pm-sysd-log --machine=dev

# 只显示指定 unit 的最近日志 (-eu 组合)
pm-sysd-log --machine=dev -eu test-run.service
```

通用选项:

```sh
-h --help
-e  # 跳到日志末尾 (默认)
-u --unit=UNIT
-x --catalog  # 初版无效果
--system
--user
-M --machine=NAME
```
