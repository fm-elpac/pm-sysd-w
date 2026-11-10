# pm-sysd-w

<https://github.com/fm-elpac/pm-sysd-w>

![CI](https://github.com/fm-elpac/pm-sysd-w/actions/workflows/ci.yml/badge.svg)

胖喵稳哒.

Systemd style on Windows.

> 本项目使用 AI (kimi, deepseek) 辅助制作.
>
> This project is made with the assistance of AI (kimi, deepseek).

pm-sysd-w 模仿 systemd 的风格和使用体验, 在 Windows 上管理进程 (service). 并不
兼容 systemd, 只实现 systemd 的一小部分功能 (subset). 配置文件采用 pm-tc
的集中式 toml 格式.

pm-sysd-w mimics the style and user experience of systemd, managing processes
(services) on Windows. It is not systemd-compatible and implements only a small
subset of systemd. Configuration uses pm-tc's centralized TOML format.

## 文档 Documentation

开发文档见 `doc/dev/`(中文) / `doc/dev_en/`(英文翻译); 用户文档见
`doc/user/`(中文) / `doc/user_en/`(英文翻译). 以中文 文档为准.

Development docs are in `doc/dev/` (Chinese) / `doc/dev_en/` (English
translation); user docs are in `doc/user/` (Chinese) / `doc/user_en/` (English
translation). The Chinese docs are authoritative.

## 快速开始 Quick Start

编译后运行, 查看命令行帮助.

Build and run to show the CLI help.

```sh
cargo build --release --features cli_init
cargo build --release --features cli_ctl
cargo build --release --features cli_log

./target/release/pm-sysd-ctl --help
```

## LICENSE

`MIT`
