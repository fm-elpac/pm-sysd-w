# pm-sysd-w 配置文件参考

pm-sysd-w 使用基于 pm-tc 的集中式 toml 格式, 多个 unit 写在一个文件里.
此处描述初版 (简单实现) 支持的配置字段.

## 顶层

```toml
pm_tc = 1
```

- `pm_tc = 1` 必填, 表示 pm-tc 配置格式版本.

- 支持 pm-tc 的 `[[include]]` 功能, 可以将主配置文件拆成多个 toml 文件, 并引用.

- `[env]` 段 (可选) 用于 pm-sysd-w 自身配置:

| 字段      | 类型   | 必填 | 默认              | 说明                                          |
| --------- | ------ | ---- | ----------------- | --------------------------------------------- |
| `SYSD_DB` | 字符串 | 否   | `pm_sysd_w_v1.db` | 本地数据库路径 (相对路径相对于主配置文件目录) |

- `SYSD_DB` 仅启动时读取, daemon-reload 不更换.

## unit (service)

每个 `[[t.sysd]]` 定义一个 unit (初版仅支持 `service` 类型). 配置通过 `env:XXX`
字段传入, 字段名与 systemd unit 文件的段/键类似.

- bool 字段支持 systemd 语法: `1` / `yes` / `on` / `true` 表示正值, `0` / `no` /
  `off` / `false` 表示负值.

### `env:Unit.*` (通用)

| 字段                             | 类型     | 必填 | 默认 | 说明                                  |
| -------------------------------- | -------- | ---- | ---- | ------------------------------------- |
| `env:Unit`                       | 字符串   | 是   | -    | unit 名称, 目前只支持 `.service` 后缀 |
| `env:Unit.Description`           | 字符串   | 否   | 空   | 描述                                  |
| `env:Unit.StartLimitIntervalSec` | 非负整数 | 否   | `10` | 启动限流时间窗口 (秒), `0` 表示不限流 |
| `env:Unit.StartLimitBurst`       | 非负整数 | 否   | `5`  | 启动限流次数, `0` 表示不限流          |

- `启动限流窗口` 内超过次数后拒绝启动 / 停止自动重启.

### `env:Service.*`

| 字段                           | 类型     | 必填 | 默认      | 说明                                                  |
| ------------------------------ | -------- | ---- | --------- | ----------------------------------------------------- |
| `env:Service.Type`             | 枚举     | 否   | `simple`  | 启动类型, 取值见下                                    |
| `env:Service.RemainAfterExit`  | bool     | 否   | `no`      | 进程退出后是否仍视为 active                           |
| `env:Service.ExecStart`        | 字符串   | 否   | -         | 启动命令 (程序路径 + 参数). 支持 `多行语法`.          |
| `env:Service.ExecStop`         | 字符串   | 否   | -         | 停止命令. 支持 `多行语法`.                            |
| `env:Service.ExecReload`       | 字符串   | 否   | -         | reload 命令. 支持 `多行语法`.                         |
| `env:Service.Restart`          | 枚举     | 否   | `no`      | 重启策略, 取值见下                                    |
| `env:Service.RestartSec`       | 非负整数 | 否   | `1`       | 重启前等待时间 (秒), 仅 Restart != no 时有效          |
| `env:Service.TimeoutStartSec`  | 非负整数 | 否   | `90`      | 启动超时 (秒), 仅 `Type=oneshot` 有效, `0` 表示不超时 |
| `env:Service.TimeoutStopSec`   | 非负整数 | 否   | `90`      | 停止超时 (秒), `0` 表示不超时                         |
| `env:Service.WorkingDirectory` | 字符串   | 否   | 继承 init | 工作目录                                              |
| `env:Service.Environment`      | 字符串   | 否   | -         | 环境变量, `NAME=VALUE`. 支持 `多行语法`.              |

`Type` 取值:

| 值        | 说明                                                                           |
| --------- | ------------------------------------------------------------------------------ |
| `simple`  | 默认, 启动即视为 active                                                        |
| `oneshot` | 进程退出 (成功) 视为启动完成, 不持续运行. `start` 命令等待 ExecStart 执行完成. |

`Restart` 取值:

| 值           | 说明                                |
| ------------ | ----------------------------------- |
| `no`         | 不自动重启 (默认)                   |
| `always`     | 退出后总是重启                      |
| `on-failure` | 仅异常退出 (非零退出码/信号) 时重启 |

---

`Exec*` 说明:

- 所有此类字段 (`ExecStart`, `ExecStop`, `ExecReload`) 都使用 pm-tc
  来切分命令行参数 (相当于 `pm-tc --exec-eval`). 不支持 shell 的引号/转义语法.

---

`ExecStart` 和 `ExecStop` 的必填: 整体原则是, 此 service 必须有意义. 具体规则为:

- `ExecStart` 和 `ExecStop` 不能同时为空. 因为这相当于此 service 什么都不做.

- 若 `Type=simple`, 则 `ExecStart` 必填. 因为 `simple`
  的语义就是一直运行某个东西, 所以需要有东西来运行.

- 若 `ExecStart` 为空, 必须同时使用 `RemainAfterExit=yes`, 此时, 此 service
  启动时立即成功. 因为, 如果 `RemainAfterExit=no`, 就相当于什么都不做.

---

`Environment` 取值: 写一个 `env:Service.Environment` 只能设置一个环境变量.
如果需要多个环境变量, 请使用 `多行语法`.

### `env:Install.*`

| 字段                   | 类型   | 必填 | 默认 | 说明                                                   |
| ---------------------- | ------ | ---- | ---- | ------------------------------------------------------ |
| `env:Install.WantedBy` | 字符串 | 否   | 空   | 所属 target, 只支持 `default.target`. 支持 `多行语法`. |

### `env:Win.*` (Windows 专属扩展)

| 字段                     | 类型 | 必填 | 默认 | 说明                                                        |
| ------------------------ | ---- | ---- | ---- | ----------------------------------------------------------- |
| `env:Win.CreateNoWindow` | bool | 否   | `no` | 启动进程时设置 `CREATE_NO_WINDOW`, console 程序不显示黑窗口 |

仅 Windows 有效, Linux 忽略.

## 多行语法

> conf 模块已实现多行语法的解析/检查. 当前 sysd 运行时暂只使用第一条 `ExecStart`
> / `ExecStop` / `ExecReload` 命令.

因为 toml 不支持重复 key, 所以为了支持类似 systemd unit 文件中, 连续写多个
(多行) `ExecStart=` 或 `Environment=` 的功能, 引入此 `多行语法`.

`多行语法` 表示依次执行多个命令, 或相应配置有多个 (比如指定多个环境变量).

比如:

| 写法 (键, key)            | 前缀 (prefix)       | 后缀 (suffix) | 备注     |
| :------------------------ | :------------------ | :------------ | :------- |
| `env:Service.ExecStart`   | `Service.ExecStart` | ``            | 空字符串 |
| `env:Service.ExecStart.1` | `Service.ExecStart` | `1`           |          |
| `env:Service.ExecStart.2` | `Service.ExecStart` | `2`           |          |
| `env:Service.ExecStart.a` | `Service.ExecStart` | `a`           |          |
| `env:Service.ExecStart.b` | `Service.ExecStart` | `b`           |          |
| `env:Service.ExecStart.c` | `Service.ExecStart` | `c`           |          |

- 写法: 在 toml 配置文件中的实际写法.

- 前缀: pm-sysd-w 支持的配置项, 本文档前面已经说明.

- 后缀: 用户自定义的部分, 使用 `.` 与前缀分隔 (前缀后面的都是后缀,
  后缀内部可以使用 `.`).

  后缀按 `字符串升序` 进行排序 (空字符串/更短的字符串排在前面).

  建议使用不会导致歧义的方式, 避免意外排序结果. 比如, 10 条以内时, 使用 `.1`,
  `.2` 这样的后缀. 100 条以内时, 使用 `.01`, `.02` 这样的后缀.

注意: 不支持 `env:Service.ExecStart.` 这样的写法 (报错),
因为此时后缀仍然是空字符串. 这种情况应该使用 `env:Service.ExecStart` 这种写法.

## 内置 target

目前仅支持内置 target, 不支持自定义:

| target           | 说明                           |
| ---------------- | ------------------------------ |
| `default.target` | init 启动后的默认 target       |
| `timers.target`  | (保留) 用于 timer              |
| `exit.target`    | init 退出时 (与所有 unit 冲突) |

## 校验限制

- pm-tc 加载的全部 toml 文件总大小不得超过 10MB, 超出立即失败.

- unit 名称最长 500 字节 (utf-8), 且必须通过 pm-tc 变量名检查. 名称必须以
  `.service` / `.timer` / `.target` 后缀结尾, 且后缀前不能为空. 初版仍只实现
  `service` 类型, `.timer` / `.target` 名称会被拒绝.

- 单个受支持的 `env:` 键值对, 键最长 500 字节 (utf-8), 值最长 20KB (utf-8).

  单个 unit 的全部受支持键值总长不得超过 500KB. 未受支持的键不参与统计.

## 示例

见 `example/pm_sysd_w.conf.toml`.

## 初版范围 (未支持)

- 不支持 unit 间依赖关系.

- 不支持自定义 `target`.

- 不支持 systemd 的 `模板 unit` 用法 (比如 `NAME@.service` 形式).

- 未列出的字段初版暂不实现.

  以 `Unit.` / `Service.` / `Install.` / `Timer.` / `Win.` 开头且未列出的字段,
  在 init 启动加载配置与 `daemon-reload` 时会写警告日志 (可用 `pm-sysd-log`
  查看), 然后忽略.

  其余字段静默忽略.
