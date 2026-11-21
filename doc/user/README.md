# pm-sysd-w 用户文档

**胖喵稳哒** 模仿 systemd 的风格和使用体验, 在 Windows 上管理进程 (process), 喵
~

使用 `pm-sysd-ctl` 操作 service, 用法类似 `systemctl`. 使用 `pm-sysd-log`
查看日志, 类似 `journalctl`.

pm-sysd-w 不兼容 systemd, 只实现其中一小部分功能 (subset). 本文档重点说明与
systemd 的 **差异**. 与 systemd 一致 (或高度相似) 的部分从简或省略.

## 文档目录

| 文档                     | 内容                         |
| ------------------------ | ---------------------------- |
| [install.md](install.md) | 编译 / 安装 / 快速开始       |
| [config.md](config.md)   | 配置文件参考                 |
| [ctl.md](ctl.md)         | `pm-sysd-ctl` 命令           |
| [log.md](log.md)         | `pm-sysd-log` 命令与日志说明 |
| [win.md](win.md)         | Windows 集成 / 专属功能      |
| [faq.md](faq.md)         | 常见问题 (暂空)              |

## 概念

`service` 是 pm-sysd-w 的一种 unit 类型, 概念类似 systemd 的 service. 注意它不是
Windows 的 "Windows 服务", 实际是 `pm-sysd-init` 启动并管理的一个 (或一组) 进程.

`pm-sysd-init` 是常驻的守护进程 (类似 systemd 的 PID 1 角色), 负责启动和管理
service. `pm-sysd-ctl` / `pm-sysd-log` 与其 (实例) 通信, 完成操作.

## 实例 (instance)

一个系统上可同时运行多个 `pm-sysd-init` 实例, 用 `pm-sysd-ctl` / `pm-sysd-log`
的实例选项选择操作目标:

| 选项             | 实例       | 说明                                                  |
| ---------------- | ---------- | ----------------------------------------------------- |
| `--system`       | 系统级实例 | 用于整台机器, 需管理员权限                            |
| `--user`         | 用户级实例 | 当前登录用户                                          |
| `--machine=NAME` | 自定义实例 | 手动启动, NAME 的字符仅限 [0-9A-Za-z_-], 长度 1..=100 |

- `pm-sysd-init` 启动时必须以 `--conf=PATH` 指定主配置文件, 并指定实例选项
  `--system` / `--user` / `--machine=NAME` 之一 (不能同时使用).

- `pm-sysd-ctl` / `pm-sysd-log` 未指定实例时, 优先尝试 `--system`,
  若系统实例未运行则回退到 `--user`. 连接 `--system` 的普通用户 (非管理员)
  自动改用只读连接. 详见 [ctl.md](ctl.md).

## 与 systemd 的主要差异

- 配置文件: 使用 `pm-tc` 的 **集中式** toml 格式, 多个 unit 写在一个文件里,
  而不是 systemd 那样的多个 unit 文件.

- 部署: 默认不包含开机自启 / 登录后自启. 项目提供 `install/setup.ps1` 作为
  **实验性** 安装方式 (见 [install.md](install.md)),
  其它自动启动方式仍需要用户手动处理.

- 默认实例: 未指定实例时优先 `--system`, 未运行则回退 `--user` (systemd
  默认只连接系统实例, 不会回退).

- 权限: 连接 `--system` 的普通用户 (非管理员) 自动改用只读连接, 只能进行只读查询
  (如查看状态 / 日志).

- `cat` 命令输出 JSON, 而不是 systemd 的 unit 文件内容.

- `soft-reboot` 是重启 `pm-sysd-init` 自身及所有被管理的 service, 不是重启
  Windows.

- Windows 关机 / 注销时无法保证优雅停止 service (详见 [win.md](win.md)).

- 日志保存在本地数据库中, 默认保留最近 72 小时. 另有专属命令 `sqlite-vacuum`
  用于清理 / 优化数据库.
