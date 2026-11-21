# CLI 模块

`cli` 模块 (仅启用 `cli` feature 时编译) 包含 3 个二进制程序的 CLI 实现, 以及 3
个程序共用的实例解析 / 连接 / 握手逻辑.

## 二进制程序与 feature

pm-sysd-w 编译出 3 个二进制程序, 每个对应 1 个 cargo feature:

| 二进制         | 角色              | feature    |
| -------------- | ----------------- | ---------- |
| `pm-sysd-init` | 类似 PID 1        | `cli_init` |
| `pm-sysd-ctl`  | 类似 `systemctl`  | `cli_ctl`  |
| `pm-sysd-log`  | 类似 `journalctl` | `cli_log`  |

默认 feature 不含它们, 应分别启用并分别编译 (命令见 `AGENTS.md`).

## 共享逻辑

实例解析 / 连接 / 握手逻辑在 `cli` 模块共用, 具体规则 (默认实例回退, 快速失败,
只读连接回退等) 见代码注释.

## 命令与输出

命令列表与选项见用户文档. 输出格式与退出码见 [sysu.md](sysu.md).

初版命令可以只输出 `TODO`, 不必全部实现.

## CLI 脚手架

依赖 `pm-bin` 提供 CLI 脚手架 (与 pm-tc 一致).
