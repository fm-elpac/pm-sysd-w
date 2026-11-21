# AGENTS.md

pm-sysd-w (胖喵稳哒): Systemd style on Windows.

模仿 systemd 的风格和使用体验, 管理 Windows 上的进程 (process).

不兼容 systemd, 只实现其中一小部分功能 (subset).

## 常用命令

```sh
cargo fmt
deno fmt

cargo build

# 测试时使用 `--all-features`
cargo test --all-features

cargo doc

# release 应分别编译 3 个 bin, 不应使用 `--all-features`
cargo build --release --features cli_init
cargo build --release --features cli_ctl
cargo build --release --features cli_log
```

## 代码约定

- 源码注释与文档注释一律使用中文. 标识符与配置关键字使用英文.

- 模块分工: `cli` (命令行程序), `cli::ctl` (pm-sysd-ctl 程序), `cli::init`
  (pm-sysd-init 程序), `cli::log` (pm-sysd-log 程序), `conf` (调用 pm-tc
  读取配置文件), `db` (sqlite 数据库抽象/封装), `jsonl` (init/ctl 通信协议),
  `logd` (日志), `pa` (跨平台抽象), `p_u` (Linux 实现, 仅用于开发测试), `p_w`
  (Windows 实现), `sysd` (服务管理核心), `sysu` (systemd 风格定义与输出格式化).

- 禁止使用 `use XXX::*` 或 `pub use XXX::*`, 必须逐个写出.

- 编译产物: 3 个二进制程序 `pm-sysd-init`, `pm-sysd-ctl`, `pm-sysd-log`.

- 源码中的 `TODO` 表示未完成, 属正常.

### 关于 `unsafe` 的使用

- 禁止 `unsafe` 代码 (`#![deny(unsafe_code)]`). 必须使用时只能放在明确允许的
  少数模块内, 且模块内代码尽量少 (能 safe 的代码不应放进来).

- 例外: `p_w::acl`, `p_w::acp`, `p_w::console`, `p_w::service::u`.

  这些 Windows API 无安全替代, 必须调用 `unsafe fn`.

  它们以 `#[allow(unsafe_code)]` 覆盖 crate 级 deny, 并把 unsafe 调用集中封装.

### panic 与错误处理

- 错误处理: 每个模块定义自己的错误枚举, 顶层错误类型用 `From` 汇总.
  错误显示仅为尽力而为.

- **禁止静默吞掉错误**: 不得用 `let _ =` 或 `.ok()` 丢弃 `Result` / 错误,
  出错时用 `unwrap` / `expect`, 或 `?` 向上传播.

  例外: `Drop` 实现与允许 `unsafe` 的模块可静默忽略错误.

  所有忽略错误或疑似静默吞错 (`let _ =`) 之处, 必须加注释说明原因.

- 关于 panic: `unwrap()` / `expect()` 只能用于 "按设计不可能失败" 的场景. 一旦
  panic 就是代码 BUG, 需要修复. 若代码 (可能) 存在 BUG, 应优先 panic (而不是 log
  错误并忽略), 以便测试阶段尽早发现.

  配置错误, 外部错误 (IPC 连接, 组件退出等) 不应直接 panic, 应返回错误, 或
  `log::error!()` 后丢弃.

### 自动化测试

- 测试代码 (rust) 应全部放到相应 `test.rs` 中, 不应有内联测试代码.

- 手动测试脚本放在 `test/` 目录, 说明见 `test/README.md`
  (例如日志写入压力测试脚本 `test/log/stress.js`).

## 文档约定

- 中文文档为准, 英文翻译由 AI 自动同步.
- 文档内容应精简. 中文文档 (以及代码注释) 也必须使用英文标点 (ASCII).
- 用户文档: `doc/user/` (中文), `doc/user_en/` (英文).
- 开发文档: `doc/dev/` (中文), `doc/dev_en/` (英文).
- 项目入口 `README.md` 采用中英混写: 一段中文后紧跟一段对应英文翻译.
- 修改代码或配置格式时, 同步更新对应文档.

---

- **开发文档** 是更高层级的设计 / 需求 / 总览, 不应重复代码注释或代码 (实现)
  本身. "一眼就能看出来" 的内容不应写入文档. 代码注释负责具体的底层细节.

### 文档风格

- 不要在汉字之间 `换行`, 只能在标点符号前/后换行. 不要使用分号 `;`, 使用逗号
  (`,`) 或句号 (`.`).

- 表格的行不要太长 (内容不要太多). 如果内容太多, 改成 `列表` 形式,
  或者在表格下方添加补充说明.

## 开发环境与辅助资料

此处描述的是 pm-sysd-w 的 "内部" 开发环境. 由于本项目开源发布,
实际开发环境可能与此不同.

### win10 测试机

如果开发环境中存在 `~/dev-vm/` 目录, 需要使用 win10 测试机时, 去阅读其中的文档
(有相关说明).

不涉及 windows 特有的修改, 应优先在本机 (Linux 开发环境) 测试, 以便加快开发.

若此目录不存在, 则是另外的开发环境, 可能没有 win10 测试机.

### systemd 源码

如果本项目的 `tmp/systemd/` 目录存在, 此处有一份准备好的 systemd 源码,
需要时可参考.
