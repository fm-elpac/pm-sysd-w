# Windows 集成 / 专属功能

pm-sysd-w 主要面向 Windows. 与 Linux (仅测试用) 相比, Windows 上多了以下内容:

- `env:Win.CreateNoWindow` 配置: 启动进程时设置 `CREATE_NO_WINDOW`, 避免 console
  程序显示黑窗口 (详见 [config.md](config.md)).

- Windows per-user 服务自动启动 (SCM 集成): `install/setup.ps1 install-user`
  会把 `pm-sysd-init --user` 安装为 per-user 服务, 用户登录时自动启动 (详见
  [install.md](install.md)).

- 普通用户只读连接 `--system` 实例 (详见 [ctl.md](ctl.md)).

## Windows 关机 / 注销时的限制

由于 Windows 的限制, 在 Windows **关机** 或用户 **注销** 时, pm-sysd-w 无法保证
(实际上几乎无法实现) 优雅停止 service:

- 在关机 / 注销阶段, Windows 不允许启动新进程 / 执行新程序, 所以 `ExecStop`
  很可能无法运行.

- 新版 Windows (Windows 10 / 11) 无法阻止关机, 系统会强制关机.
  而且每个程序只有很短的固定硬超时上限 (比如 5 秒). 也就是说, Windows
  最多等待每个程序 5 秒, 然后就会强制关机. 这个行为无法配置, 程序也无法修改.
