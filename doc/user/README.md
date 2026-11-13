# pm-sysd-w 用户文档

pm-sysd-w (胖喵稳哒): Systemd style on Windows.

使用 `pm-sysd-ctl` 管理 Windows 上的进程 (service), 用法类似 `systemctl`.

注意: 本文档中的 `service` 不是 Windows 的 service (Windows 服务), 而是
pm-sysd-w 的一种 unit 类型, 概念类似 systemd 的 service. 它实际表现为由
`pm-sysd-init` 启动的一个 (或一组) 进程.

## 安装

需要 Rust 工具链 (edition 2024).

仅支持 Windows 10 或更新版本.

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

注意: 目前版本不包含 开机自动启动 (作为 Windows 服务运行) 和 用户登录后自动运行
等功能; 这些都需要用户手动处理. 例如, pm-sysd-w 不会自动把 `pm-sysd-init` 安装
为 Windows 服务, 也不会自动设置登录后自启; 它只负责 启动并管理 service.

`--system` / `--user` / `--machine=NAME` 三个实例选项 不能同时使用,
只能指定一个.

`pm-sysd-ctl` / `pm-sysd-log` 未指定实例选项时, 会优先尝试 `--system`; 若系统
实例未运行, 则自动回退到 `--user`. 这与 systemd 的默认行为不同. 连接 `--system`
时, 普通用户 (非管理员) 会自动改用只读连接, 只能进行 只读查询 (如查看日志 /
状态).

## Windows 关机 / 注销 时的限制

由于 Windows 的限制, 在 Windows 关机 或 用户注销 时, pm-sysd-w 无法保证 (实际上
几乎无法实现) 优雅停止 service:

- 在 关机 / 注销 阶段, Windows 不允许 启动新进程 / 执行新程序, 所以 `ExecStop`
  很可能无法运行.
- 新版 Windows (Windows 10 / 11) 无法阻止关机, 系统会强制关机; 而且 每个程序
  只有很短的固定硬超时上限 (比如 5 秒). 也就是说, Windows 最多等待 每个程序 5
  秒, 然后就会强制关机. 这个行为 无法配置, 程序也无法修改.

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
  target); 只有值为 `default.target` 的 unit 才能 `enable`.
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

命令行为说明:

- unit 名称未写后缀时, 自动补 `.service` (例如 `status test` 等价于
  `status test.service`); `pm-sysd-log` 的 `-u` / `--unit` 同理.
- `cat` 输出 pretty JSON (不模仿 systemd 的 unit 文件格式).
- `is-active` 退出码: `0` active, `3` 非 active (inactive/failed), `4` 未知.
- `is-failed` 退出码: `0` failed, `1` 非 failed, `4` 未知.
- `is-enabled` 退出码: `0` enabled, `1` disabled/masked, `4` 未知.
- `set-default` 初版仅支持 `default.target`; 其它值报错.
- `list-dependencies` 因 unit 间依赖功能尚未实现, 返回空结果.

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

## 日志说明

- 日志默认保留 最近 72 小时, pm-sysd-init 每 1 小时自动清理一次.
- service 进程的 stdout / stderr 会按行写入对应 unit 的日志. 单行超长时会被
  拆分为多条日志; 输出不保证 UTF-8, 解码失败时该行会换成一条 error 日志写入对应
  unit.

## 优雅退出

- `pm-sysd-ctl ... exit` 正常退出 pm-sysd-init.
- `pm-sysd-ctl ... soft-reboot` 重启 pm-sysd-init 自身及所有被管理 service (不是
  Windows 重启).
- 外部信号: Linux 上 SIGTERM, Windows 上 Ctrl+C, 也会触发优雅退出.
