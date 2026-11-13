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

仅支持 Windows 10 或更新版本.

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

注意: 本文档中的 `service` 不是 Windows 的 service (Windows 服务), 而是
pm-sysd-w 的一种 unit 类型, 概念类似 systemd 的 service. 它实际表现为由
`pm-sysd-init` 启动的一个 (或一组) 进程.

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

| 模块         | 说明                                                                        |
| ------------ | --------------------------------------------------------------------------- |
| `cli`        | 命令行程序 公共模块 (仅 cli feature)                                        |
| `cli::init`  | 实现 `pm-sysd-init` 程序 (CLI): 命令行参数解析 / 帮助, soft-reboot 实际执行 |
| `cli::ctl`   | 实现 `pm-sysd-ctl` 程序 (CLI)                                               |
| `cli::log`   | 实现 `pm-sysd-log` 程序 (CLI)                                               |
| `jsonl`      | init 与 ctl 之间的 JSONL 通信协议                                           |
| `conf`       | 调用 pm-tc 读取 toml 配置文件                                               |
| `db`         | sqlite 数据库 抽象/封装                                                     |
| `sysd`       | 服务管理 核心逻辑                                                           |
| `sysd::init` | init 主循环 (pm-sysd-init 常驻逻辑)                                         |
| `logd`       | 日志 核心逻辑 (类似 journald, 简化实现)                                     |
| `pa`         | 底层平台 (操作系统) 抽象接口                                                |
| `p_w`        | Windows (主要平台) 的 pa 实现                                               |
| `p_u`        | Linux (仅测试) 的 pa 实现                                                   |

## 配置文件 (依赖 pm-tc)

pm-sysd-w 不使用 systemd 那种 "一堆配置文件" 的方式, 而是采用 **集中式** 配置:
把很多 unit 的配置写在一个 toml 配置文件中, 由 pm-tc (胖喵模板配置工具) 读取.

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

未列出的 `env:XXX` 字段: 以 `Unit.` / `Service.` / `Install.` / `Timer.` /
`Win.` 开头 且未支持的字段, `conf` 模块会收集为警告, 由 init 启动加载配置 与
`daemon-reload` 两处调用 `log::warn!()` 输出, 并写入日志 (pm-sysd-log 可查看),
然后忽略; 其余字段 静默忽略.

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
3. 双方各自比较对方 `data.version` 是否与自己一致; 不一致则报错. 其中 init
   在版本不一致时, 会先向对方发送自己的版本 (握手消息), 然后关闭连接.

只有握手消息携带版本号, 后续消息无需版本号.

后续消息中, ctl / log 发给 init 的 `SdReq` 为两层结构: 外层 `req` 字段 (`sys-d`
/ `log-ro` / `unitd-ro` / `unit-d` / `unit-ro` / `unit`) 表示处理组件与
是否只读, 内层 `data` 对象中的 `op` 字段表示具体操作. 只读查询与修改操作
在数据结构上直接区分. 例如:

```json
{ "req": "unit", "data": { "name": "a.service", "op": "start" } }
```

其中, 多个 ctl 命令若调用 init 的同一接口, 只对应一种 `op`; 具体输出语义由 ctl
自己负责.

### 防御性限制

pm-sysd-init 对 IPC 连接做如下防御 (常量定义在 `jsonl` / `sysd::init` 模块中,
方便后续修改):

- 单条消息 最大 1MB (`jsonl::MAX_MESSAGE_LEN`); 超出 报错 (`log::error!()`)
  并关闭连接.
- 单条消息 收 / 发 各自必须在 1 秒内完成 (`jsonl::RECV_TIMEOUT` /
  `jsonl::SEND_TIMEOUT`); 超时 报错并关闭连接.
- pm-sysd-init 最大并发连接 200 (`sysd::init::MAX_CONNECTIONS`);
  超出报错并关闭连接.
- 单条消息 解析失败 (协议级对不上的错误数据): 报错并关闭连接.
- 握手消息 无法解析: 报错并关闭连接; 版本不一致: init 发送自己的版本后关闭 连接.
- 数据读写 使用缓冲块 (`fill_buf` / `consume`, `write_all`), 不逐字节读写.

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

注意: 目前版本不包含 开机自动启动 (作为 Windows 服务运行) 和 用户登录后自动运行
等功能; 上述启动方式 需要用户手动配置 (例如 把 `pm-sysd-init` 安装为 Windows
服务).

pm-sysd-init 启动参数 (命令行):

- `--system`: 运行系统级实例.
- `--user`: 运行当前用户实例.
- `--machine=XXX`: 运行用户自定义名称实例.
- `--conf=PATH`: 指定主配置文件 (pm-tc toml) 路径 (必填).

`--system` / `--user` / `--machine=XXX` 启动时必选其一, 且 不能同时指定多个;
同时指定多个 会 报错退出.

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

数据库访问全部通过 `db` 模块完成: 模块内启动 2 个专用 (同步) 线程 (数据库
读写线程 / 日志写入线程), 只有这两个线程直接访问 sqlite. tokio 的 async 线程
通过 MPSC 向数据库读写线程发送请求, 请求中携带响应 MPSC, 读写线程完成后回传;
async 线程因此不直接执行 sqlite IO, 也不会被慢速数据库 IO 阻塞.

### 启动流程

pm-sysd-init 启动按如下顺序:

0. 命令行参数 解析 / 处理.
1. 实例唯一性检查 (早于其余所有操作), 失败直接退出; 成功后即持有 UNIX socket /
   命名管道, 防止 TOCTOU. Windows 上同时按文档设置命名管道 ACL; `--system`
   实例还会创建 `_ro` 只读命名管道 (也是唯一性检查的一部分, 失败同样退出).
2. 加载配置文件; 失败直接退出.
3. 打开数据库.
4. 其余后续处理 (创建 SysD, 异步 启动默认 target 中的 service, 进入 IPC 主循环).

### 内部机制: 多 actor 架构

pm-sysd-init 的 `sysd` 模块采用多 actor 架构. 每个关键组件是 `tokio::spawn` 的
async task, 持有私有状态, 单线程 处理自己的状态; 组件之间通过 MPSC 消息通信.
关键组件 (`SysD` / `UnitD` / `ServiceD` / `ServiceS` / `Logd`) 都是可 clone 的
句柄 struct: 通过 `spawn()` (Logd 通过 `Logd::new`) 启动对应 task. 多数组件通过
`req()` 发送消息并等待回执; Logd 的日志查询请求通过 `req_log_ro()` 转发,
只等待投递完成, 不等待处理/回执.

消息与状态机命名约定 (M 结尾 = 消息, SM 结尾 = 状态机):

- `SysD` (`sysd::sys_d`): 顶层实体, 只负责命令路由. 自己只处理 `SdReq::SysD`
  (退出 / soft-reboot); `SdReq::LogRO` (日志查询) 转发给 Logd, 其余 ctl 请求与
  `StartDefaultTarget` 原样转发给 UnitD. 转发消息不阻塞主循环: 每条消息自带
  oneshot 回执, 转发时直接透传, 只有处理 `SdReq::SysD` 时等待. 退出时先通知
  UnitD 停止所有 unit, 再通过 `exit_tx` 通知 `sysd::init`, 然后退出自己的循环.
- `UnitD` (`sysd::unit_d`): 管理所有 unit (内存配置) 并启动 ServiceD; 处理
  daemon-reload, ctl 请求, 以及 service 生命周期编排. 状态机 `UnitDSM`
  (`sysd::unit_d::actor`) 持有内存数据库 `UnitS` (`sysd::unit_d::unit_s`),
  负责决策并输出动作; 配置读取 / 持久化在 `load` (`sysd::unit_d::load`),
  查询合成 task 在 `query` (`sysd::unit_d::query`).
- `ServiceD` (`sysd::service_d`): 管理所有 service; 状态封装在 `ServiceDSM`
  (`sysd::service_d::actor`) 中, 查询逻辑在 `query` (`sysd::service_d::query`).
  ServiceS 的创建 (新启动) 交给 ServiceD 的 job 队列 task 串行执行, 主循环只
  入队 start job.
- `ServiceS` (`sysd::service_s`): 管理单个 service; 状态封装在 `ServiceSSM`
  (`sysd::service_s::actor`) 中. ServiceS 只持有 SM 与其它组件的引用,
  不直接持有状态. 对外 API 为 `spawn()` / `send()`, 消息 `ServiceSM` 自带
  oneshot 回执.
- `Logd` (`logd`): 日志记录器. 日志查询请求 (`SdReq::LogRO`) 由 后台 task 处理
  并回复; 写入 / 测试用查询 (`write_with` / `recent_entries` / `page_entries`)
  直接使用数据库客户端.
- `TimerD` / `TimerS` (`sysd::timer_d` / `timer_s`): 预留.

组件之间可发送的消息 (如 `SysDM` / `UnitDM` / `ServiceDM` / `ServiceSM`, 以及
Logd 的日志查询请求) 是相应模块的 pub API; 其它组件直接构造消息数据并发送. ctl
请求 (jsonl 模块的 `SdReq*` 类型) 转发时尽量整体透传, 不逐枚举值重新包装. `SysD`
/ `UnitD` / `ServiceD` / `ServiceS` 的消息自带 oneshot 回执; SysD / UnitD
转发时直接透传, 因此不阻塞自己的主循环.

### 内部机制: 事件驱动

service 生命周期命令 (start / stop / restart / reload / kill 等) 由 UnitD
状态机读取 unit 类型后转发给 ServiceD, 并由 ServiceD 直接回复调用方; ServiceD /
ServiceS 各自单线程串行处理消息. ServiceS 的创建 (新启动) 交给 ServiceD 的 job
队列 task 串行执行, 主循环只入队 start job. 单个 unit 操作不会阻塞 UnitD 主循环.

service 管理为事件驱动: 子进程退出由监视任务 `await` 子进程退出后发消息;
定时重启由定时任务发消息. 运行中的 ServiceS 在启动时快照 unit 配置,
整个生命周期保持不变, 因此 daemon-reload 后运行中的 service 仍使用旧配置, 不需要
unit gc 机制.

### 优雅退出 与 soft-reboot

- 退出 / soft-reboot 由 SysD 决定: 收到 `SdReq::SysD` (或外部信号转换成的 exit
  命令) 后, 先让 UnitD 停止所有 service, 再通过 `exit_tx` 通知 init. 外部信号
  (SIGTERM / Ctrl+C) 也统一转换为发给 SysD 的 exit 命令.
- 退出后 init 记录日志并等待日志落库.
- `soft-reboot` 在完成全部清理工作后, 使用相同命令行参数重启自身: Linux 上
  `exec` 执行自己; Windows 上此时已放弃独占命名管道, 启动新的 `pm-sysd-init.exe`
  后自行退出.

### 日志 防御规则

- 单条日志 最大 500KB (`db::MAX_LOG_LEN`); 发送方 自动把超长日志拆分为多条.
- 发送方 投递失败 (接收队列已满) 时 丢弃并 `log::warn!()` 警告; 警告做速率 限制.
- `db` 模块的日志写入线程 遇到超长日志 丢弃并警告 (防御性检查).
- 读取 subprocess 的 stdout / stderr: 单行长度受日志最大长度限制, 超长拆分为
  多条日志. stdout / stderr 不保证 UTF-8; 解码失败时 换成一条 error 日志,
  写入对应 unit.
- `pm-sysd-log` 的日志读取接口为 "分页" 接口, 返回 总量 / 游标, 方便灵活读取,
  同时适配 jsonl 1MB 单条消息限制.
- 日志保留: 每 1 小时自动清理一次, 只保留最近 72 小时 (`logd` 模块常量).

### 命名管道权限测试 (手动)

`test/test-permission.ps1` 用于在 Windows 上 手动验证 命名管道 ACL 权限.
需管理员 运行, 且需先编译二进制:

```sh
cargo build --features cli_init
cargo build --features cli_ctl
powershell -ExecutionPolicy Bypass -File test\test-permission.ps1
```

脚本会 启动一个 `--system` 实例, 创建临时标准用户, 分别以 管理员 / 普通用户
尝试连接 主管道 (`pm_sysd_w_system_default`) 与 只读管道
(`pm_sysd_w_system_default_ro`), 校验: 普通用户 不能连接 主管道, 但 能连接
只读管道. 该脚本 必须手动运行 (自动化测试 不覆盖 跨用户权限).

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

### 默认实例选择 (与 systemd 不同)

`pm-sysd-ctl` / `pm-sysd-log` 未显式指定实例时, 优先尝试 `--system`; 若
`--system` 不存在 (未运行), 则回退到 `--user`.

这与 systemd 的默认行为不同: `systemctl` 默认只连接系统实例, 不会自动回退.
pm-sysd-w 增加回退, 是考虑到 Windows 上 普通用户 常常没有 系统实例, 而更常用
用户实例.

### 连接失败 (快速失败)

`pm-sysd-ctl` / `pm-sysd-log` 连接 init 时, 先检查 IPC 端点 (命名管道 / UNIX
socket) 是否存在; 存在之后才尝试连接, 而不是直接连接. 连接重试的总耗时 最多
200ms, 让 init 不存在时 快速报错退出, 不等待过久.

Windows 上连接 `--system` 时, 优先连接主管道 `pm_sysd_w_system_default`;
若因权限不足失败, 再回退连接只读管道 `pm_sysd_w_system_default_ro` (普通用户
只能进行 只读查询).

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

- unit 类型 计划只支持 `service` / `timer` / `target` 三种; 当前版本 只实现
  `service`, 不实现 `timer`, 也不支持 自定义 `target`.
- 不支持 unit 间依赖关系.
- 不支持自定义 target.

## 项目约定

- 源码注释 / 文档注释 一律使用中文 (可以含空格), 标识符 / 配置关键字为英文.
- 禁止 `unsafe` 代码 (`#![deny(unsafe_code)]`). 例外: `p_w::console` 与
  `p_w::acl` 两个模块 (Windows API 无安全替代, 必须调用 `unsafe fn`), 它们以
  `#[allow(unsafe_code)]` 覆盖 crate 级 deny, 并把 unsafe 调用集中封装.
- 错误处理: 每个模块定义自己的错误枚举, 顶层错误类型用 `From` 汇总; 错误显示仅为
  尽力而为, 便于开发调试.

- **禁止静默吞掉错误**: 一般不得用 `let _ =` 或 `.ok()` 丢弃 `Result` / 错误;
  出错时使用 `unwrap` / `expect`, 或 `?` 向上传播. 例外: `Drop` 实现与允许
  `unsafe` 的模块 (`p_w::console` / `p_w::acl`) 可静默忽略错误.
  所有忽略错误或疑似静默吞错 (`let _ =`) 之处, 必须加注释说明原因.

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
