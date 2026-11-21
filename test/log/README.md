# 日志写入压力测试 test/log/

本目录存放日志写入压力测试脚本. 测试生成包含多个 service 的临时配置, 用
`pm-sysd-init --machine` 启动, 让 service 持续向 stdout / stderr 输出日志,
再对比实际产出与 sqlite 入库情况, 观察日志写入, 丢弃, 内存占用与吞吐.

## 文件

| 文件           | 作用                                               |
| -------------- | -------------------------------------------------- |
| `stress.js`    | 主运行器, 负责执行场景, 收集结果, 打印汇总         |
| `scenarios.js` | 场景定义 (默认, 调优, 超长, 限速)                  |
| `emit.js`      | service 执行的子进程脚本, 支持全速与限速输出       |
| `lib.js`       | 公共工具: 进程运行, 数据库读取, 内存采样, 配置生成 |

## 准备

```sh
cargo build --release --features cli_init
cargo build --release --features cli_ctl
```

依赖 node.js (要求支持 `node:sqlite`, 本机为 node v26).

## 运行

```sh
node test/log/stress.js
```

常用选项:

| 选项                                    | 说明                                  |
| --------------------------------------- | ------------------------------------- |
| `--type=default\|tune\|long\|rate\|all` | 选择测试类型, 默认 `default`          |
| `--duration=N`                          | 覆盖 default/tune/long 场景的持续秒数 |
| `--scenario=A,B`                        | 只运行指定场景 (default/tune/long)    |
| `--rate-duration=N`                     | 限速搜索中每次运行的秒数, 默认 10     |
| `--rate-iters=N`                        | 限速二分搜索最大迭代次数, 默认 12     |
| `--sample-ms=N`                         | 内存采样间隔, 默认 200 毫秒           |
| `--no-index`                            | 测试全程关闭数据库日志表索引          |
| `--keep`                                | 保留临时目录用于排查                  |

- `--no-index` 会在生成的临时配置 `[env]` 中写入 `SYSD_DB_LOG_NO_INDEX = "1"`,
  新建数据库的日志表不创建索引.

- 对于通过环境变量传给 `pm-sysd-init` 的配置 (例如 tune 场景的 `SYSD_LIMIT_*`),
  如果外部进程环境中已存在同名变量, 脚本直接使用外部值并输出警告提示.
  这样可以不改脚本, 从外部覆盖内部默认值, 方便调整测试条件.

- 在 Windows 的 cmd 里用 `set` 设置上述环境变量时, 应写成 `set "VAR=value"`.
  `set VAR=value && 命令` 会把 `value` 连同 `&&` 前的空格一起设为变量值 (实际为
  `"value "` 带尾随空格), 导致 Rust 解析该值时失败并回退到默认值.

临时目录位于项目 `tmp/` 下, 不占用 tmpfs. 脚本默认运行完删除, `--keep` 可保留.

## 测试类型

### default: 默认配置全速输出

使用默认日志限制, 多个 service 同时全速输出日志. 场景 `s1` / `s4` / `s8` / `s16`
对应 1 / 4 / 8 / 16 个 service, `s4-long` 为长行 (1000B). 运行期间以
`--sample-ms` 间隔采样 `pm-sysd-init` 的内存占用 (RSS), 报告最小 / 最大 /
平均值, 用于观察全速写入时 init 内存的波动.

### tune: 调整日志缓冲与内存

在 `s8` 全速输出基础上, 通过给 `pm-sysd-init` 传入
`SYSD_LIMIT_LOG_PENDING_BYTE`, 逐档放大待写日志数据总大小上限, 观察是否改善
丢日志情况. 相关环境变量默认值见 `src/conf/limit.rs`:

| 环境变量                            | 默认值          | 作用                       |
| ----------------------------------- | --------------- | -------------------------- |
| `SYSD_LIMIT_LOG_PENDING_BYTE`       | 10485760 (10MB) | 待写日志数据总大小上限     |
| `SYSD_LIMIT_LOG_PENDING_N`          | 100000          | 日志批发线程缓冲最大条数   |
| `SYSD_LIMIT_LOG_CHANNEL_CAPACITY_N` | 1000            | 日志输入 MPSC 通道容量     |
| `SYSD_LIMIT_DB_CHANNEL_CAPACITY_N`  | 1000            | 数据库读写线程请求缓冲条数 |

本目录内置 3 组调优场景 `tune-s8-b2` / `tune-s8-b4` / `tune-s8-b8`, 分别把
`SYSD_LIMIT_LOG_PENDING_BYTE` 放大到 20MB / 40MB / 80MB (默认的 2 / 4 / 8 倍),
其余日志限制保持默认值.

### long: 超长单行日志

单行日志长度从 100KB 到 1MB. `lineBytes` 为每行填充字节, 实际行长还会加上 label
与计数前缀. 超过默认单行长度限制 (`SYSD_LIMIT_LOG_LINE_BYTE = 500 * 1024`)
的行会被拆分为多条入库. 这类场景以字节为统计单位, 报告产出字节, 入库字节,
字节丢弃率与拆分系数 (入库条数 / 产出行数),
用于观察超长行写入性能与拆分是否正确.

### rate: 限速搜索

用 1 个 service 输出短行日志, 二分搜索在 0% 丢失前提下的最高日志输出速率
(字节/秒). 脚本先全速测一次得到上限, 再在 `[上限/1000, 上限]` 区间内二分,
每次运行 `--rate-duration` 秒, 最后报告达到 0% 丢失的最高速率, 并做一次确认运行.
每次尝试同样采样内存并输出吞吐量 (`条/s` 与 `MB/s`).

## 结果解读

- `produced`: service 实际输出的日志条数 (default/tune/rate) 或负载字节 (long).
- `stored`: sqlite 中 `level IN ('stdout','stderr')` 的条数或 `msg` 总字节.
- `dropped` / `drop%`: 产出减去入库, 及丢弃百分比.
- `produced/s` / `stored/s`: 吞吐量, 同时输出 `条/s` 与字节速率 (`MB/s` 等),
  不再只输出其中一种.
- `rss min/max/avg`: `pm-sysd-init` 进程 RSS 采样结果, 单位见 `fmtBytes`.
- `db size`: sqlite 文件大小.

汇总表中的 `lines/s` 即 `条/s`, `MB/s` 为入库字节吞吐.

## Windows 兼容性

脚本代码已按跨平台编写: 数据库读取使用 `node:sqlite`, 内存采样在 Windows 上用
`powershell (Get-Process -Id PID).WorkingSet64`. 注意两点:

- pm-tc 的 `ExecStart` 按空白切分且不支持引号,
  因此仓库路径与临时目录路径不能包含空格. 脚本把临时目录放在项目 `tmp/` 下,
  并把传给 service 的路径转换为正斜杠形式.
- Windows 上每采一次内存都会启动一次 powershell, 速度较慢, 可调大 `--sample-ms`.

目前主要在本机 (Linux) 测试, Windows 分支也已在真实 Windows 10 机器上跑过完整
压力测试 (结果见 `doc/dev/log.md`).
