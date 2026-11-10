# AGENTS.md

pm-sysd-w (胖喵稳哒): Systemd style on Windows.

模仿 systemd 的风格和使用体验, 管理 Windows 上的进程 (service). 不兼容 systemd,
只实现其中一小部分功能 (subset).

配置文件使用 pm-tc (胖喵模板配置工具) 的集中式 toml 格式.

## 常用命令

```sh
cargo fmt
deno fmt
cargo build
cargo test
cargo doc
cargo build --release
```

## 代码约定

- 源码注释 与 文档注释 一律使用中文 (可以含空格). 标识符与配置关键字使用英文.
- 禁止 `unsafe` 代码 (`#![deny(unsafe_code)]`).
- 模块分工: `cli` (命令行程序 公共模块), `cli::init` (pm-sysd-init 程序),
  `cli::ctl` (pm-sysd-ctl 程序), `cli::log` (pm-sysd-log 程序), `jsonl`
  (init/ctl 通信协议), `conf` (调用 pm-tc 读取配置文件), `db` (sqlite 数据库
  抽象/封装), `sysd` (服务管理核心), `logd` (日志 核心), `pa` (平台抽象接口),
  `p_w` (Windows 实现), `p_u` (Linux 实现, 仅测试).
- 编译产物: 3 个二进制程序 `pm-sysd-init`, `pm-sysd-ctl`, `pm-sysd-log`.
- 错误处理: 每个模块定义自己的错误枚举, 顶层错误类型用 `From` 汇总; 错误显示仅为
  尽力而为.
- 源码中的 `TODO` 表示未完成, 属正常.

## 文档约定

- 中文 文档为准, 英文翻译 由 AI 自动同步.
- 文档 内容应该精简. 中文 文档 (以及 代码注释) 也必须使用 英文标点 (ASCII).
- 开发文档: `doc/dev/` (中文), `doc/dev_en/` (英文).
- 用户文档: `doc/user/` (中文), `doc/user_en/` (英文).
- 项目入口 `README.md` 采用中英混写: 一段中文后紧跟一段对应英文翻译.
- 修改代码或配置格式时, 同步更新对应文档.
