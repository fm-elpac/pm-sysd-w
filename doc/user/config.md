# pm-sysd-w 配置文件参考

pm-sysd-w 使用 pm-tc 的集中式 toml 格式, 多个 unit 写在一个文件里. 本页描述 初版
(简单实现) 支持的配置字段. 内容为草稿, 后续可能调整.

## 顶层

```toml
pm_tc = 1
```

`pm_tc = 1` 必填, 表示 pm-tc 配置格式版本.

`[env]` 段 (可选) 用于 pm-sysd-w 自身设置:

| 字段      | 类型   | 必填 | 默认              | 说明                                                                               |
| --------- | ------ | ---- | ----------------- | ---------------------------------------------------------------------------------- |
| `SYSD_DB` | 字符串 | 否   | `pm_sysd_w_v1.db` | sqlite 数据库路径 (相对路径相对主配置文件目录); 仅启动时读取, daemon-reload 不更换 |

## unit (service)

每个 `[[t.sysd]]` 定义一个 unit (初版仅支持 `service` 类型). 配置通过 `env:XXX`
字段传入, 字段名与 systemd unit 文件的段/键 类似.

bool 字段支持 systemd 布尔语法: `1` / `yes` / `true` / `on` 表示 正值, `0` /
`no` / `false` / `off` 表示 负值.

### `env:Unit.*` (通用)

| 字段                   | 类型   | 必填 | 默认 | 说明                            |
| ---------------------- | ------ | ---- | ---- | ------------------------------- |
| `env:Unit`             | 字符串 | 是   | -    | unit 名称, 须以 `.service` 结尾 |
| `env:Unit.Description` | 字符串 | 否   | 空   | 描述                            |

### `env:Service.*`

| 字段                           | 类型     | 必填 | 默认      | 说明                                         |
| ------------------------------ | -------- | ---- | --------- | -------------------------------------------- |
| `env:Service.Type`             | 枚举     | 否   | `simple`  | 启动类型, 取值见下                           |
| `env:Service.ExecStart`        | 字符串   | 是   | -         | 启动命令 (程序路径 + 参数)                   |
| `env:Service.ExecStop`         | 字符串   | 否   | -         | 停止命令                                     |
| `env:Service.ExecReload`       | 字符串   | 否   | -         | reload 命令                                  |
| `env:Service.Restart`          | 枚举     | 否   | `no`      | 重启策略, 取值见下                           |
| `env:Service.RestartSec`       | 非负整数 | 否   | `1`       | 重启前等待时间 (秒), 仅 Restart != no 时有效 |
| `env:Service.TimeoutStartSec`  | 非负整数 | 否   | `90`      | 启动超时 (秒)                                |
| `env:Service.TimeoutStopSec`   | 非负整数 | 否   | `90`      | 停止超时 (秒)                                |
| `env:Service.WorkingDirectory` | 字符串   | 否   | 继承 init | 工作目录                                     |
| `env:Service.Environment`      | 字符串   | 否   | -         | 环境变量, `KEY=VALUE`                        |
| `env:Service.RemainAfterExit`  | bool     | 否   | `no`      | 进程退出后是否仍视为 active                  |

`Type` 取值 (初版):

| 值        | 说明                                     |
| --------- | ---------------------------------------- |
| `simple`  | 默认, 启动即视为 active                  |
| `oneshot` | 进程退出 (成功) 视为启动完成, 不持续运行 |

`Restart` 取值 (初版简单实现):

| 值           | 说明                                |
| ------------ | ----------------------------------- |
| `no`         | 不自动重启 (默认)                   |
| `always`     | 退出后总是重启                      |
| `on-failure` | 仅异常退出 (非零退出码/信号) 时重启 |

### `env:Install.*`

| 字段                   | 类型   | 必填 | 默认 | 说明                                                       |
| ---------------------- | ------ | ---- | ---- | ---------------------------------------------------------- |
| `env:Install.WantedBy` | 字符串 | 否   | 空   | 所属 target (仅内置 target); 显式设置后该 unit 才可 enable |

### `env:Win.*` (pm-sysd-w 专属)

| 字段                     | 类型 | 必填 | 默认 | 说明                                                                                   |
| ------------------------ | ---- | ---- | ---- | -------------------------------------------------------------------------------------- |
| `env:Win.CreateNoWindow` | bool | 否   | `no` | 启动进程时设置 CREATE_NO_WINDOW, console 程序不显示黑窗口; 仅 Windows 有效, Linux 忽略 |

## 内置 target

初版仅支持内置 target, 不支持自定义:

| target           | 说明                              |
| ---------------- | --------------------------------- |
| `default.target` | init 启动后的默认 target          |
| `timers.target`  | (保留) 用于 timer                 |
| `exit.target`    | init 退出时 (与所有 service 冲突) |

## 示例

见 `example/pm_sysd_w.conf.toml`.

## 初版范围 (未支持)

- 仅 `service` unit 类型; 不支持 timer 等其它类型.
- 不支持 unit 间依赖关系.
- 不支持自定义 target.
- 未列出的字段初版暂不实现 (可忽略或保留).
