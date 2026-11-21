# sysd 模块 (服务管理核心)

`sysd` 是服务管理核心逻辑. 初版只支持 service 一种 unit 类型.

## 多 actor 架构

pm-sysd-init 的 `sysd` 采用多 actor 架构: 每个关键组件是 `tokio::spawn` 的 async
task, 持有内部状态, 单线程接收并处理消息, 组件间通过 MPSC 通信.

组件句柄均可 clone. 消息发送为无阻塞 (只等待投递完成, 不等待处理 / 回执).

命名约定: `M` 结尾为消息, `SM` 结尾为状态机. 每个组件由 actor (执行 IO) 与状态机
(纯计算决策) 组成.

## 组件

| 组件              | 职责                         |
| ----------------- | ---------------------------- |
| `sysd::init`      | init 主循环 (常驻逻辑)       |
| `sysd::sys_d`     | 顶层实体, 只做命令路由       |
| `sysd::unit_d`    | 管理所有 unit, 启动 ServiceD |
| `sysd::service_d` | 管理所有 service             |
| `sysd::service_s` | 管理单个 service             |
| `sysd::timer_d`   | 管理所有 timer (预留)        |
| `sysd::timer_s`   | 单个 timer (预留)            |

各组件职责与消息 / 状态机细节见代码注释.

## 实例唯一性

同一实例 (同名 IPC 端点) 只能有 1 个进程, 重复启动报错退出. 唯一性检测,
即创建并持有 IPC 端点, 早于其它所有操作, 避免 TOCTOU.

## 配置加载

使用 pm-tc 的集中式配置 (`[[t.sysd]]`). 配置加载与 mask/enable 状态读取由
`sysd::reload` 统一处理, 启动加载与 daemon-reload 共用. 配置来源抽象为
`Reloader` trait, 生产实现 `SelfReloader` 通过 `pm-sysd-init --reload` 子进程
读取配置, 测试可注入自定义 Reloader 跳过该子进程.

## 事件驱动

service 生命周期为事件驱动: 子进程退出 / 定时器到期等由后台任务发消息, 单个 unit
操作不阻塞主循环. 运行中的 service 在启动时快照配置, daemon-reload
后仍使用旧配置.

## 优雅退出与 soft-reboot

退出 / soft-reboot 由 SysD 决定: 先停止所有 service, 再通知 init.
退出后记录日志, 等待落库, 并对数据库执行一次 vacuum. soft-reboot
用相同命令行参数重启自身.

启动流程 (命令行解析, 唯一性检测, 配置加载, 数据库打开, 启动 default.target,
进入 IPC 主循环) 见 `sysd::init` 代码注释.
