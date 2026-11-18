# pm-sysd-w

<https://github.com/fm-elpac/pm-sysd-w>

![CI](https://github.com/fm-elpac/pm-sysd-w/actions/workflows/ci.yml/badge.svg)

胖喵稳哒.

Systemd style on Windows.

> 本项目使用 AI (kimi, deepseek) 辅助制作.
>
> This project is made with the assistance of AI (kimi, deepseek).

pm-sysd-w 模仿 systemd 的风格和使用体验, 在 Windows 上管理进程 (process). 并不
兼容 systemd, 只实现 systemd 的一小部分功能 (subset). 配置文件采用 pm-tc
的集中式 toml 格式.

pm-sysd-w mimics the style and user experience of systemd, managing processes on
Windows. It is not systemd-compatible and implements only a small subset of
systemd. Configuration uses pm-tc's centralized TOML format.

## 举栗 Example

比如, 配置文件 `test.conf.toml`:

For example, a config file `test.conf.toml`:

```toml
pm_tc = 1

[[t.sysd]]
"env:Unit" = "test-ping.service"
"env:Unit.Description" = "Ping 127.0.0.1 test"

"env:Service.ExecStart" = "ping 127.0.0.1"

"env:Service.Restart" = "always"
"env:Service.RestartSec" = "20"

"env:Install.WantedBy" = "default.target"
```

`ping.exe` 默认 4 次后退出, 配合 `Restart=always` 即每 20 秒开始 ping.

`ping.exe` exits after 4 pings by default, so with `Restart=always` it starts a
ping every 20 seconds.

首先, 启动用户级实例 (守护进程):

First, start a user instance (the daemon):

```sh
pm-sysd-init.exe --user --conf=test.conf.toml
```

启动服务, 查看状态, 停止服务:

Start the service, show its status, and stop it:

```sh
pm-sysd-ctl --user start test-ping
pm-sysd-ctl --user status test-ping
pm-sysd-ctl --user stop test-ping
```

在 `pm-sysd-init` 启动后, 自动启动服务:

After `pm-sysd-init` starts, the service starts automatically:

```sh
pm-sysd-ctl --user enable test-ping
```

查看日志:

View logs:

```sh
pm-sysd-log --user -eu test-ping.service
```

退出用户实例:

Exit the user instance:

```sh
pm-sysd-ctl --user exit
```

## 快速开始 Quick Start

编译后运行, 查看命令行帮助.

Build and run to show the CLI help.

```sh
cargo build --release --features cli_init
cargo build --release --features cli_ctl
cargo build --release --features cli_log

./target/release/pm-sysd-ctl --help
```

## 文档 Documentation

用户文档见 `doc/user/`(中文) / `doc/user_en/`(英文翻译). 开发文档见
`doc/dev/`(中文) / `doc/dev_en/`(英文翻译). 以中文文档为准.

User docs are in `doc/user/` (Chinese) / `doc/user_en/` (English translation).
Development docs are in `doc/dev/` (Chinese) / `doc/dev_en/` (English
translation). The Chinese docs are authoritative.

## LICENSE

`MIT`
