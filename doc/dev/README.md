# pm-sysd-w 开发文档

## 阅读说明

本文档面向开发者, 记录用户文档 (`doc/user/`) 未包含的开发 / 内部内容.

- 开发者必须先阅读用户文档. 本文档在其基础上补充实现细节,
  不重复用户文档已有的内容.

- 与 systemd 一致 (或高度相似) 的部分, 本文档从简或省略. 重点记录本项目与
  systemd 的 **差异**.

## 项目定位

pm-sysd-w (胖喵稳哒) 模仿 systemd 的风格和使用体验, 管理 Windows 上的进程
(process).

不兼容 systemd, 只实现其中一小部分功能 (subset). 与 systemd 的差异,
及用户可见行为, 见用户文档.

名称 "稳哒" 来自网络名句 "稳稳哒接住你" (狗头

## 编译目标平台

pm-sysd-w 支持 2 个编译目标:

| 目标    | 三元组                     | 用途           |
| ------- | -------------------------- | -------------- |
| Windows | `x86_64-pc-windows-msvc`   | 主要支持平台   |
| Linux   | `x86_64-unknown-linux-gnu` | 仅用于方便测试 |

仅支持 Windows 10 或更新版本.

Linux 目标仅用于方便测试: Linux 已有 systemd, pm-sysd-w 在 Linux
上几乎没有实际用途. 在 Linux 上能测试的只是 pm-sysd-w 全部功能的一部分
(可能测不全), Windows 上的测试才覆盖全部功能. 由于在 Windows 上测试较为麻烦,
开发过程中先在 Linux 上快速验证, 之后再到 Windows 上做完整验证.

## 二进制程序与 feature

pm-sysd-w 编译出 3 个二进制程序:

| 程序           | 角色              | 说明                                                      |
| -------------- | ----------------- | --------------------------------------------------------- |
| `pm-sysd-init` | 类似 "PID 1"      | 常驻程序, 管理由自己启动的进程 (service), 基于 tokio 异步 |
| `pm-sysd-ctl`  | 类似 `systemctl`  | 主要命令行工具, 通过与 `pm-sysd-init` 交互完成操作, 同步  |
| `pm-sysd-log`  | 类似 `journalctl` | 日志查看工具, 通过与 `pm-sysd-init` 交互读取日志, 同步    |

3 个二进制程序分别对应 3 个 cargo feature:

| 二进制         | feature    |
| -------------- | ---------- |
| `pm-sysd-init` | `cli_init` |
| `pm-sysd-ctl`  | `cli_ctl`  |
| `pm-sysd-log`  | `cli_log`  |

默认 feature 不包含这 3 个 feature. 正常情况下, 应分别启用对应的 feature
并分别编译 (不应同时启用 2 个 cli feature):

```sh
cargo build --release --features cli_init
cargo build --release --features cli_ctl
cargo build --release --features cli_log
```

这样 `pm-sysd-init --version`, `pm-sysd-ctl --version`, `pm-sysd-log --version`
的输出会不同 (pm-bin 的 `--version` 会显示启用的所有 feature).

## 模块结构

| 模块         | 说明                                                                        |
| ------------ | --------------------------------------------------------------------------- |
| `cli`        | 命令行程序公共模块 (仅 cli feature)                                         |
| `cli::init`  | 实现 `pm-sysd-init` 程序 (CLI): 命令行参数解析 / 帮助, soft-reboot 实际执行 |
| `cli::ctl`   | 实现 `pm-sysd-ctl` 程序 (CLI)                                               |
| `cli::log`   | 实现 `pm-sysd-log` 程序 (CLI)                                               |
| `jsonl`      | init 与 ctl 之间的 JSONL 通信协议                                           |
| `conf`       | 调用 pm-tc 读取 toml 配置文件                                               |
| `db`         | sqlite 数据库抽象/封装                                                      |
| `sysd`       | 服务管理核心逻辑                                                            |
| `sysd::init` | init 主循环 (pm-sysd-init 常驻逻辑)                                         |
| `sysu`       | systemd 风格的状态定义 / 打印输出 (模仿 systemd)                            |
| `sysu::ctl`  | pm-sysd-ctl 的 systemctl 风格输出与退出码                                   |
| `sysu::log`  | pm-sysd-log 的 journalctl 风格输出                                          |
| `logd`       | 日志核心逻辑 (类似 journald, 简化实现)                                      |
| `pa`         | 底层平台 (操作系统) 抽象接口                                                |
| `p_w`        | Windows (主要平台) 的 pa 实现                                               |
| `p_u`        | Linux (仅测试) 的 pa 实现                                                   |

## 配置文件 (依赖 pm-tc)

pm-sysd-w 使用集中式配置, 由 pm-tc (胖喵模板配置工具) 读取 (与 systemd 的分散
unit 文件不同).

加载配置后, pm-sysd-w 调用 pm-tc 的 `task::plan("sysd")`, 从返回的 `PlanDebug`
中拿到所有 unit 配置. 每个 unit 对应配置文件中的一个 `[[t.sysd]]` 任务实例, 通过
`env:XXX` 字段传递 unit 配置. 配置字段及示例见用户文档 (`doc/user/config.md`).

未列出的 `env:XXX` 字段: 以 `Unit.` / `Service.` / `Install.` / `Timer.` /
`Win.` 开头且未支持的字段, `conf` 模块会收集为警告, 由 init 启动加载配置与
`daemon-reload` 两处调用 `log::warn!()` 输出, 并写入日志 (`pm-sysd-log` 可查看),
然后忽略. 其余字段静默忽略.

## 通信 (IPC)

pm-sysd-init 与 pm-sysd-ctl / pm-sysd-log 的通信方式:

- Linux: 使用当前目录的 UNIX socket (仅用于方便测试, 权限默认, 无需专门处理).

- Windows: 使用命名管道 (Named pipe), 需要严格权限控制. 只允许本机程序连接,
  禁止网络连接.

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

3. 双方各自比较对方 `data.version` 是否与自己一致, 不一致则报错. 其中 init
   在版本不一致时, 会先向对方发送自己的版本 (握手消息), 然后关闭连接.

只有握手消息携带版本号, 后续消息无需版本号.

后续消息中, ctl / log 发给 init 的 `SdReq` 为两层结构: 外层 `req` 字段 (`sys-d`
/ `log-ro` / `unitd-ro` / `unit-d` / `unit-ro` / `unit`) 表示处理组件与是否只读,
内层 `data` 对象中的 `op` 字段表示具体操作.

只读查询与修改操作在数据结构上直接区分. 例如:

```json
{ "req": "unit", "data": { "name": "a.service", "op": "start" } }
```

其中, 多个 ctl 命令若调用 init 的同一接口, 只对应一种 `op`. 具体输出语义由 ctl
自己负责.

### 内存目标

pm-sysd-init 的期望运行时内存 (最大) 为 100MB. 也就是说, 在只有 100MB
可用内存时, pm-sysd-init 仍应能正常运行, 不会崩溃.

因此, 代码中会引入各种防御性限制, 避免内存占用过大.

### 防御性限制

pm-sysd-init 对 IPC 连接做如下防御 (常量定义在 `pa` / `sysd::init` 模块中,
方便后续修改):

- 单条消息最大 10MB (`pa::MAX_MESSAGE_LEN`). 超出报错 (`log::error!()`)
  并关闭连接.

- 单条消息发送必须在 1 秒内完成 (`pa::SEND_TIMEOUT`). 超时报错并关闭连接.

- 单条消息接收超时 (`pa::RECV_TIMEOUT`) 从收到本条消息 `第一个字节` 开始,
  到本条消息 `最后一个字节` 结束. 对方完全没发消息时不超时. 超时报错并关闭连接.

- pm-sysd-init 最大并发连接 200 (`sysd::init::MAX_CONNECTIONS`).
  超出报错并关闭连接.

- 单条消息解析失败 (协议级对不上的错误数据): 报错并关闭连接.

- 握手消息无法解析: 报错并关闭连接. 版本不一致: init 发送自己的版本后, 关闭连接.

- 数据读写使用缓冲块 (`fill_buf` / `consume`, `write_all`), 不逐字节读写.

传输协议属于内部 API, 不作为外部稳定 API 提供, 小版本更新也可能引入 BREAKING
change. 原则上 init 与 ctl / log 的版本应一致.

## pm-sysd-init

实例的概念与用户可见行为见用户文档. 本节只记录内部实现.

允许一个 Windows 系统上同时运行多个 pm-sysd-init 实例. 使用命名管道进行
`实例唯一性检测` (同名实例只能有 1 个进程, 重复启动则报错退出).

命名管道名称前缀: `pm_sysd_w_`.

| 实例          | 命名管道                              | 权限                   |
| ------------- | ------------------------------------- | ---------------------- |
| 系统级        | `pm_sysd_w_system_default`            | 仅管理员               |
| 系统级 (只读) | `pm_sysd_w_system_default_ro`         | 本机普通用户可只读查询 |
| 用户级        | `pm_sysd_w_user_NAME` (NAME 为用户名) | 仅当前用户             |
| 自定义        | `pm_sysd_w_m_XXX`                     | 仅当前用户             |

pm-sysd-init 启动参数 (命令行) 及其校验规则见用户文档.

### 数据库

pm-sysd-init 使用 sqlite 数据库存储持久状态 (例如某个 unit 是否 enable, 是否
mask) 与日志. 数据库格式属于内部 API, 不作为外部稳定 API 提供. 数据库文件位置与
`SYSD_DB` 配置见用户文档, schema 详见 [db.md](db.md).

数据库访问全部通过 `db` 模块完成: 模块内启动 2 个专用 (同步) 线程
(数据库读写线程 / 日志写入线程), 只有这两个线程直接访问 sqlite. tokio 的 async
线程通过 MPSC 向数据库读写线程发送请求, 请求中携带响应 oneshot,
读写线程完成后回传. async 线程因此不直接执行 sqlite IO, 也不会被慢速数据库 IO
阻塞.

### 启动流程

pm-sysd-init 启动按如下顺序:

0. 命令行参数解析 / 处理.

1. 实例唯一性检查 (早于其余所有操作), 失败直接退出. 成功后即持有 UNIX socket /
   命名管道, 防止 TOCTOU. Windows 上同时按文档设置命名管道 ACL. `--system`
   实例还会创建 `_ro` 只读命名管道 (也是唯一性检查的一部分, 失败同样退出).
   唯一性检测完成后, 继续启动前调用 pm-bin 的 `print_version()` 输出版本信息.

2. 加载配置文件 (通过 `pm-sysd-init --reload` 子进程执行). 加载超过 1 秒
   未完成会输出警告并继续等待, 超过 2 秒未完成会终止子进程并按失败处理,
   直接退出.

3. 打开数据库.

4. 其余后续处理 (创建 SysD, 启动默认 target 中的 service, 然后进入 IPC 主循环).

   默认 target 启动完成后输出 `[  OK  ] Reached target default.target.`

### 内部机制: 多 actor 架构

pm-sysd-init 的 `sysd` 模块采用多 actor 架构. 每个关键组件是 `tokio::spawn` 的
async task, 持有内部状态, 单线程 (async) 接收并处理消息. 组件之间通过 MPSC
消息通信.

关键组件 (`SysD` / `UnitD` / `ServiceD` / `ServiceS` / `Logd`) 都是可 clone
的句柄 struct: 通过 `spawn()` (Logd 通过 `Logd::new`) 启动对应 task.

多数组件通过 `send()` 无阻塞发送消息, 只等待投递完成, 不等待处理/回执.

消息与状态机命名约定 (`M` 结尾 = 消息, `SM` 结尾 = 状态机):

- `SysD` (`sysd::sys_d`): 顶层实体, 只负责命令路由.

  自己只处理 `SdReq::SysD` (exit / soft-reboot / sqlite-vacuum). `SdReq::LogRO`
  (日志查询) 转发给 Logd, 其余 ctl 请求与 `StartDefaultTarget` 原样转发给 UnitD.

  转发消息不阻塞主循环: 每条消息自带 oneshot 回执, 转发时直接透传.
  `sqlite-vacuum` 由独立 task 投递给数据库读写线程, 同样不阻塞主循环. 只有处理
  `SdReq::SysD` 中的 exit / soft-reboot 时等待.

  退出时先通知 UnitD 停止所有 unit, 再通过 `exit_tx` 通知 `sysd::init`,
  然后退出自己的循环.

- `UnitD` (`sysd::unit_d`): 管理所有 unit (内存配置) 并启动 ServiceD.

  处理 daemon-reload, ctl 请求, 以及 service 生命周期编排.

  状态机 `UnitDSM` (`sysd::unit_d::actor`) 持有内存数据库 `UnitS`
  (`sysd::unit_d::unit_s`), 负责 决策并输出动作.

  配置与 mask/enable 状态的读取由 `load` (`sysd::init::load`) 统一处理
  (pm-sysd-init 启动与 daemon-reload 共用); enable/mask 持久化在 actor
  (`sysd::unit_d::actor`), 查询合成 task 在 `query` (`sysd::unit_d::query`).

- `ServiceD` (`sysd::service_d`): 管理所有 service.

  状态封装在 `Sm` (`sysd::service_d::sm`) 中, actor (`sysd::service_d::actor`)
  只执行状态机输出的 IO 请求, 查询逻辑在 `query` (`sysd::service_d::query`).

  ServiceS 由 ServiceD actor 直接创建 / 启动: actor 先同步登记句柄与 active
  状态, 再投递初始命令. ServiceS 不自行退出, 需要退出时向 ServiceD 发送退出请求,
  由 ServiceD 回发 `ServiceSM::Exit`.

- `ServiceS` (`sysd::service_s`): 管理单个 service.

  状态封装在 `ServiceSSM` (`sysd::service_s::actor`) 中. ServiceS 只持有 SM
  与其它组件的引用, 不直接持有 状态.

  对外 API 为 `spawn()` / `send()`, 消息 `ServiceSM` 自带 oneshot 回执.

- `Logd` (`logd`): 日志记录器.

  日志查询请求 (`SdReq::LogRO`) 由后台 task 处理并回复. 写入 / 测试用查询
  (`write_with` / `page_entries`) 直接使用数据库客户端.

- `TimerD` / `TimerS` (`sysd::timer_d` / `timer_s`): 预留.

---

组件之间可发送的消息 (如 `SysDM` / `UnitDM` / `ServiceDM` / `ServiceSM`, 以及
Logd 的日志查询请求) 是相应模块的 pub API, 其它组件直接构造消息数据并发送.

ctl 请求 (jsonl 模块的 `SdReq*` 类型) 转发时尽量整体透传, 不逐枚举值重新包装.

`SysD` / `UnitD` / `ServiceD` / `ServiceS` 的消息自带 oneshot 回执, SysD / UnitD
转发时直接透传, 因此不阻塞自己的主循环.

### 内部机制: 事件驱动

service 生命周期命令 (start / stop / restart / reload / kill 等) 由 UnitD
状态机读取 unit 类型后转发给 ServiceD, 并由 ServiceD 直接回复调用方. ServiceD /
ServiceS 各自单线程串行处理消息. ServiceS 由 ServiceD actor 直接创建 / 启动,
单个 unit 操作不会阻塞 UnitD 主循环.

service 管理为事件驱动: 子进程退出由监视任务 `await` 子进程退出后发消息.
定时重启由定时任务发消息.

运行中的 ServiceS 在启动时快照 unit 配置, 整个生命周期保持不变. 因此
daemon-reload 后运行中的 service 仍使用旧配置, 不需要 unit gc 机制.

### 优雅退出与 soft-reboot

- 退出 / soft-reboot 由 SysD 决定: 收到 `SdReq::SysD` (或外部信号转换成的 exit
  命令) 后, 先让 UnitD 停止所有 service, 再通过 `exit_tx` 通知 init. 外部信号
  (SIGTERM / Ctrl+C) 也统一转换为发给 SysD 的 exit 命令.

  所有 service 停止后输出 `[  OK  ] Reached target exit.target.`

- 退出后 init 记录日志, 等待日志落库, 然后 (soft-reboot 重启自身前) 对数据库
  执行一次 vacuum.

- `soft-reboot` 在完成全部清理工作后, 使用相同命令行参数重启自身: Linux 上
  `exec` 执行自己. Windows 上此时已放弃独占命名管道, 启动新的 `pm-sysd-init.exe`
  后自行退出.

### 日志防御规则

- 单条日志最大 500KB (`logd::MAX_LOG_LEN`). 发送方自动把超长日志拆分为多条.

- 发送方投递失败 (接收队列已满) 时丢弃并 `log::warn!()` 警告. 警告做速率限制.

- `db` 模块的日志写入线程遇到超长日志丢弃并警告 (防御性检查).

- 读取 subprocess 的 stdout / stderr: 读取缓冲上限为单条日志最大长度,
  超长行拆分为多条日志, 不会因单行过长无限增长缓冲.

  解码使用 `pa::TextDecoder` 流式解码: Linux 固定 UTF-8. Windows 先按 UTF-8,
  失败后切换到系统 ACP, ACP 再失败时换成一条 error 日志, 写入对应 unit.

- `pm-sysd-log` 的日志读取接口为 `分页` 接口, 返回总量 / 游标.

  查询请求通过 `asc` 指定方向 (`true` 升序 / `false` 降序), 通过 `pos` (`Start`
  / `End` / `Cursor`) 指定开始位置. 返回条数由 `asc` 与 `limit` 决定.
  数据库直接按此方向排序返回, `logd` 与 jsonl 响应保持同一顺序, 不再 `reverse`.

  `logd` 先用估算选择本页日志, 再用 encode 实测兜底, 保证单页不超过 jsonl
  单条消息最大长度限制. 数据库读取侧也会按累计日志内容长度提前停止,
  避免一次读入过大. `pm-sysd-log` 通过游标多次读取直至凑够所需条数.

- 单次分页查询最多 10000 条 (`logd::LOG_PAGE_MAX`). 外部请求超过时自动截断.

- 日志保留: 每 1 小时自动清理一次, 只保留最近 72 小时 (`logd` 模块常量).

### 命名管道权限测试 (手动)

`test/test-permission.ps1` 用于在 Windows 上手动验证命名管道 ACL 权限.
需管理员运行, 且需先编译二进制:

```cmd
cargo build --features cli_init
cargo build --features cli_ctl

powershell -ExecutionPolicy Bypass -File test\test-permission.ps1
```

脚本会启动一个 `--system` 实例, 创建临时标准用户, 分别以管理员 /
普通用户尝试连接主管道 (`pm_sysd_w_system_default`) 与只读管道
(`pm_sysd_w_system_default_ro`), 校验: 普通用户不能连接主管道, 但能连接只读管道.
该脚本必须手动运行 (自动化测试不覆盖跨用户权限).

## pm-sysd-ctl / pm-sysd-log

命令列表与选项见用户文档. 本节只记录内部实现.

模仿 systemctl / journalctl 的风格 (包括输出格式), 但不使用 pager. 只是相似,
不追求完全一致. 输出中不要出现 `systemd` 等词, 因为本项目并不是 systemd.

所有命令 CLI 都要有, 但初版不一定实际实现功能, 可以只输出 `TODO`,
也不必支持更多参数/选项.

- `list-units` 为分页接口: 请求携带 `limit` 与游标 (最后一个 unit 名称),
  响应携带总量 / 下一页游标 / 本页 unit.

  UnitD 组合当前配置与 ServiceD 中仍在运行的 service (含已被 mask 但仍在运行的
  service, 此类显示为 masked), 按 unit 名称排序.

  返回前对整条响应做 JSON encode 实测, 太长时按 "超限减半" 减少本页条数
  (至少保留 1 条), 保证不超出 jsonl 单条消息限制.

  `pm-sysd-ctl` 自动循环拉取全部页后打印.

### 连接失败 (快速失败)

`pm-sysd-ctl` / `pm-sysd-log` 连接 init 时, 先检查 IPC 端点 (命名管道 / UNIX
socket) 是否存在. 存在之后才尝试连接, 而不是直接连接. 连接重试的总耗时最多
200ms, 让 init 不存在时快速报错退出, 不等待过久.

Windows 上连接 `--system` 时, 优先连接主管道 `pm_sysd_w_system_default`.
若权限不足失败, 再回退连接只读管道 `pm_sysd_w_system_default_ro`
(普通用户只能进行只读查询).

默认实例选择 (`--system` 优先, 回退 `--user`) 的行为与理由见用户文档.

## install/setup.ps1 (安装脚本)

`install/setup.ps1` 是实验性的安装/卸载脚本, 提供 `install-user` 与
`uninstall-user` 两个命令. 它直接使用 PowerShell 调用 Windows 工具, 不依赖
pm-sysd-w 自身的 Rust 代码.

需要管理员权限 (安装/删除服务会写 `HKLM`).

### install-user

按顺序执行:

1. 复制 `pm-sysd-init.exe` / `pm-sysd-ctl.exe` / `pm-sysd-log.exe`. 查找顺序为:
   当前目录, 当前目录的 `target/release/`, 脚本所在目录的上级目录, 及其
   `target/release/`. 找到后复制到 `%LOCALAPPDATA%\pm-sysd-w\bin\`.

2. 用 `[Environment]::GetEnvironmentVariable('Path', 'User')` 读取当前用户
   `PATH`, 若目标目录不在其中, 则追加并用
   `[Environment]::SetEnvironmentVariable('Path', ..., 'User')` 写回.

3. 若 `%LOCALAPPDATA%\pm-sysd-w\pm_sysd_w.conf.toml` 不存在, 用
   `[System.IO.File]::WriteAllText` 写入空配置 `pm_tc = 1`. `conf::load_conf`
   允许没有 `[[t.sysd]]` 的配置, 因此该文件可正常加载.

4. 用 `sc.exe create pm_sysd_w_user type= userown start= auto binPath= ...` 把
   `pm-sysd-init.exe --user --conf=...` 注册为 per-user 服务模板. 其 `Type` 为
   `SERVICE_USER_OWN_PROCESS` (0x50), 服务模板位于
   `HKLM\SYSTEM\CurrentControlSet\Services\pm_sysd_w_user`.

   per-user 服务由 Windows SCM 管理: 用户登录时, SCM 根据模板创建
   `pm_sysd_w_user_<LUID>` 实例并启动. 用户注销时停止并删除该实例.

### uninstall-user

`sc.exe delete pm_sysd_w_user` 删除服务模板, 不删除 `%LOCALAPPDATA%\pm-sysd-w\`
目录.

### pm-sysd-init 的 Windows 服务集成

`pm-sysd-init` 启动时 (仅 Windows) 先尝试连接 SCM
(`StartServiceCtrlDispatcherW`): 成功表示由 SCM 启动, 进入服务模式. 返回
`ERROR_FAILED_SERVICE_CONTROLLER_CONNECT` 表示普通命令行启动, 继续原有流程.
服务模式实现见 `p_w::service`:

- 服务主函数注册控制处理回调, 上报 `SERVICE_START_PENDING` / `SERVICE_RUNNING`
  等状态, 避免 SCM 以 "未及时响应" (错误 1053) 终止服务.

- 收到 `SERVICE_CONTROL_STOP` / `SERVICE_CONTROL_SHUTDOWN` 时置位停止事件.
  `pa::spawn_signal_watch` 在服务模式下等待该事件, 触发与 Ctrl+C 相同的优雅退出.

### 已知限制

- 服务模板是机器级 (HKLM) 配置, 但脚本把 exe 放在当前用户的 `%LOCALAPPDATA%`.
  因此该安装方式目前只适用于单用户场景, 其它用户无法直接复用该模板.

- 服务模式下 `soft-reboot` 不自行重启进程 (按正常退出处理), 避免脱离 SCM 管理.

## 项目约定

关于编写代码的具体要求, 详见 `AGENTS.md` (`代码约定` 章节),
以及源代码各模块中的注释, 此处不再重复. 因为本项目主要使用 AI 写代码,
人类主要只做架构设计和代码审核.

- 依赖 `pm-bin` 提供 CLI 脚手架 (与 pm-tc 一致).

### 代码 (文档) 格式化

```sh
cargo fmt  # rust 代码
deno fmt  # 其余文件 (比如 markdown)
```

## 构建与测试

详见 `AGENTS.md` (`常用命令` 章节), 以及 CI (`.github/workflows/ci.yml`).
