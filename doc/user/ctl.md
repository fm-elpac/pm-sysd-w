# pm-sysd-ctl 命令

`pm-sysd-ctl` 操作 service, 风格与 `systemctl` 相似.

未指定实例时, 优先尝试 `--system`, 若系统实例未运行则回退到 `--user`. 连接
`--system` 时, 普通用户 (非管理员) 自动改用只读连接, 只能进行只读查询.

## 命令列表

(初版) 支持的命令, 部分命令初版可能只输出 `TODO`:

```sh
list-units
list-timers

status
is-active
is-failed
show
cat
list-dependencies

start
stop
restart
try-restart
reload
reload-or-restart
try-reload-or-restart
kill
reset-failed
clean

enable
disable
is-enabled
reenable
preset
mask
unmask
get-default
set-default

daemon-reload
whoami
is-system-running

show-environment
set-environment
unset-environment
import-environment

# 重启 pm-sysd-init 自身及所有被管理 service
soft-reboot
# 退出 pm-sysd-init
exit

# 清理/优化 sqlite 数据库
sqlite-vacuum
```

## 通用选项

```sh
   --version
-h --help

   --system
   --user
-M --machine=NAME

   --wait
   --no-block
```

## 命令行为说明

- unit 名称未写后缀时, 自动补 `.service` (例如 `status test` 等价于
  `status test.service`).

- `list-units` / `status` / `list-timers` 的输出格式尽量模仿 `systemctl`,
  与本项目无关的字段省略.

- `list-units` 按 unit 名称排序. `daemon-reload` 后已从新配置移除但仍运行的 unit
  也会列出 (状态来自运行中的旧配置).

- `enable` / `disable` 支持 `--now` (例如 `enable --now`).

- `preset` 初版等效于 `disable`.

- `mask` 行为与 systemd 一致: 支持 `--now` (默认只 mask, 加 `--now` 才停止). 被
  mask 的 unit 视为不存在, 但仍运行中的 service 会以 `masked` 状态显示, 且
  `start` 等命令会拒绝启动.

- `is-active`, `is-failed`, `is-enabled` 退出码与 systemd 一致.

- `is-system-running` 输出整个实例的运行状态, 退出码模仿 systemd: `running` 返回
  0, 其余状态返回 1. 启动初期输出 `initializing` 或 `starting`, 退出时输出
  `stopping`, 管理器未运行时输出 `offline`, 查询状态失败时输出 `unknown`.
  本项目暂不使用 `maintenance`.

- `whoami` 会把请求发给 pm-sysd-init 处理. 带一个 PID 参数时, 查询该 PID 是否
  属于正在运行的 service; 命中则输出 `Service=名称`, 未命中输出错误并返回非零.
  不带参数时的含义与 systemctl 不同: 返回 pm-sysd-init 自身信息 (`InstanceType`,
  `InstanceName`, `Pid`, 作为 Windows 服务运行时还有 `ServiceName`). 默认多行
  `KEY=VALUE` 输出; 加 `--json` 改为单行 JSON, key 与文本格式相同.

- `show` 与 `cat` 输出相同的全部受支持配置字段: `show` 为 `key=value` (key
  与配置一致, 如 `Service.ExecStart`), `cat` 为 pretty JSON (不模仿 systemd 的
  unit 文件格式).

- `list-dependencies` 因 unit 间依赖功能尚未实现, 返回空结果.

- `daemon-reload` 重新读取配置文件并刷新 enable/mask 状态. reload 期间执行
  `enable` / `disable` / `reenable` / `mask` / `unmask` 会报错,
  且不允许并发执行多个 `daemon-reload`.

- `set-default` 初版仅支持 `default.target`, 其它值报错.
