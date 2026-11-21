# 文档 Documentation

`胖喵稳哒` 的文档.

Documentation of `pm-sysd-w`.

## 中文

用户文档 (入口: [user/README.md](user/README.md)):

- [install.md](user/install.md): 编译 / 安装 / 快速开始.
- [config.md](user/config.md): 配置文件参考.
- [ctl.md](user/ctl.md): `pm-sysd-ctl` 命令.
- [log.md](user/log.md): `pm-sysd-log` 命令与日志说明.
- [win.md](user/win.md): Windows 集成 / 专属功能.
- [faq.md](user/faq.md): 常见问题 (暂空).

开发文档 (入口: [dev/README.md](dev/README.md)):

- [jsonl.md](dev/jsonl.md): init 与 ctl / log 的 IPC / JSONL 协议.
- [limit.md](dev/limit.md): 防御机制 / 内存目标.
- [db.md](dev/db.md): sqlite 数据库抽象与 schema.
- [log.md](dev/log.md): 日志核心逻辑 (`logd` 模块).
- [sysd.md](dev/sysd.md): 服务管理核心 (`sysd` 模块).
- [sysu.md](dev/sysu.md): systemd 风格输出 (`sysu` 模块).
- [cli.md](dev/cli.md): CLI 模块 (3 个二进制程序).
- [win.md](dev/win.md): Windows 集成 (`pa` / `p_w` / `p_u` 模块).
- [unsafe.md](dev/unsafe.md): `unsafe` 代码说明.

测试脚本见 `test/` 目录, 入口: [test/README.md](../test/README.md).

## English translation

User docs (entry: [user_en/README.md](user_en/README.md)):

- [install.md](user_en/install.md): Build / install / quick start.
- [config.md](user_en/config.md): Config reference.
- [ctl.md](user_en/ctl.md): `pm-sysd-ctl` commands.
- [log.md](user_en/log.md): `pm-sysd-log` commands and log notes.
- [win.md](user_en/win.md): Windows integration / exclusive features.
- [faq.md](user_en/faq.md): FAQ (empty for now).

Developer docs (entry: [dev_en/README.md](dev_en/README.md)):

- [jsonl.md](dev_en/jsonl.md): IPC / JSONL protocol between init and ctl / log.
- [limit.md](dev_en/limit.md): Defensive mechanisms / memory target.
- [db.md](dev_en/db.md): SQLite database abstraction and schema.
- [log.md](dev_en/log.md): Core logging logic (`logd` module).
- [sysd.md](dev_en/sysd.md): Core service management (`sysd` module).
- [sysu.md](dev_en/sysu.md): systemd-style output (`sysu` module).
- [cli.md](dev_en/cli.md): CLI module (3 binaries).
- [win.md](dev_en/win.md): Windows integration (`pa` / `p_w` / `p_u` modules).
- [unsafe.md](dev_en/unsafe.md): `unsafe` code notes.

Test scripts are in the `test/` directory, entry:
[test/README.md](../test/README.md).

---

以中文文档为准.

The Chinese docs are authoritative.
