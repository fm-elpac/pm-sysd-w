# 编译 / 快速开始 / 安装

## 编译

目前需要自行编译. 需要 Rust 工具链 (<https://rust-lang.org/>), 仅支持 Windows 10
或更新版本.

```sh
cargo build --release --features cli_init
cargo build --release --features cli_ctl
cargo build --release --features cli_log
```

编译产物位于 `target/release/` 目录: `pm-sysd-init.exe`, `pm-sysd-ctl.exe`,
`pm-sysd-log.exe`.

## 快速开始

可以暂不安装, 手动运行 `pm-sysd-init` 快速体验. 安装方式见 `安装脚本` 章节.

启动一个实例 (比如用户实例 `--user`), 指定主配置文件 (见 [config.md](config.md))
的绝对路径:

```sh
pm-sysd-init --user --conf=path/to/pm_sysd_w.conf.toml
```

再用 `pm-sysd-ctl` 操作 (见 [ctl.md](ctl.md)), 比如:

```sh
pm-sysd-ctl --user start test-run

pm-sysd-ctl --user status test-run

pm-sysd-ctl --user stop test-run

pm-sysd-ctl --user enable test-run
```

查看日志 (见 [log.md](log.md)), 退出实例:

```sh
pm-sysd-log --user -eu test-run.service

pm-sysd-ctl --user exit
```

## 安装脚本 (实验性)

项目提供 `install/setup.ps1`, 在项目根目录用 PowerShell 运行. 目前支持
`install-user` 与 `uninstall-user` 两个命令, 需要管理员权限.

```powershell
# 在项目根目录运行
powershell -ExecutionPolicy Bypass -File install\setup.ps1 install-user
powershell -ExecutionPolicy Bypass -File install\setup.ps1 uninstall-user
```

`install-user` 会:

1. 把 `pm-sysd-init.exe` / `pm-sysd-ctl.exe` / `pm-sysd-log.exe` 复制到
   `%LOCALAPPDATA%\pm-sysd-w\bin\`.

2. 把此目录加入当前用户 `PATH`.

3. 在 `%LOCALAPPDATA%\pm-sysd-w\pm_sysd_w.conf.toml` 写入空配置文件
   (已存在则不覆盖).

4. 把 `pm-sysd-init.exe --user` 安装为 Windows per-user 服务
   (用户登录时自动启动).

---

`uninstall-user` 只删除 per-user 服务, 保留 `%LOCALAPPDATA%\pm-sysd-w\` 目录.

> **实验性支持**: 目前仅支持这两个命令. `pm-sysd-init` 实现了 Windows 服务集成
> (SCM), per-user 服务会随用户登录自动启动. 其它自动启动方式 (任务计划程序 /
> 启动文件夹 / Run 注册表项) 仍由用户手动处理.

## 退出与重启实例

- `pm-sysd-ctl exit` 正常退出 pm-sysd-init.

- `pm-sysd-ctl soft-reboot` 重启 pm-sysd-init 自身, 及所有被管理 service (不是
  Windows 重启).

- 外部信号: Windows 上 `Ctrl+C`, Linux 上 `SIGTERM`, 也会触发优雅退出.
