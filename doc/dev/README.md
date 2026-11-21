# pm-sysd-w 开发文档

## 阅读说明

本文档面向开发者, 记录用户文档 (`doc/user/`) 未包含的开发 / 内部内容.

- 开发者必须先阅读用户文档. 本文档在其基础上补充 **设计 / 需求 / 总览**,
  不重复用户文档已有的内容.

- 与 systemd 一致 (或高度相似) 的部分, 本文档从简或省略. 重点记录本项目与
  systemd 的 **差异**.

- 本文档不重复代码注释, 也不重复代码 (实现) 本身. 具体底层实现细节 (常量取值,
  消息类型, 状态机状态, 表结构等) 见代码注释.

## 项目定位

pm-sysd-w (胖喵稳哒) 模仿 systemd 的风格和使用体验, 管理 Windows 上的进程
(process).

不兼容 systemd, 只实现其中一小部分功能 (subset). 与 systemd 的差异,
及用户可见行为, 见用户文档.

名称 "稳哒" 来自网络名句 "稳稳哒接住你" (狗头

## 编译目标平台

pm-sysd-w 支持 2 个编译目标:

| 目标    | 三元组                     | 用途           |
| ------- | -------------------------- | -------------- |
| Windows | `x86_64-pc-windows-msvc`   | 主要支持平台   |
| Linux   | `x86_64-unknown-linux-gnu` | 仅用于方便测试 |

仅支持 Windows 10 或更新版本.

Linux 目标仅用于方便测试: Linux 已有 systemd, pm-sysd-w 在 Linux
上几乎没有实际用途. 在 Linux 上能测试的只是 pm-sysd-w 全部功能的一部分
(可能测不全), Windows 上的测试才覆盖全部功能. 由于在 Windows 上测试较为麻烦,
开发过程中先在 Linux 上快速验证, 之后再到 Windows 上做完整验证.

## 模块结构

- `cli`: 命令行程序公共模块与 3 个 CLI 实现 (仅 cli feature). 详见
  [cli.md](cli.md).

- `jsonl`: init 与 ctl / log 之间的 JSONL 通信协议. 详见 [jsonl.md](jsonl.md).

- `conf`: 调用 pm-tc 读取 toml 配置文件. 加载说明见 [sysd.md](sysd.md).

- `db`: sqlite 数据库抽象/封装. 详见 [db.md](db.md).

- `logd`: 日志核心逻辑 (类似 journald, 简化实现). 详见 [log.md](log.md).
  日志压力测试与参数实验见 [log_test.md](log_test.md).

- `sysd`: 服务管理核心逻辑 (含 `sysd::init` 常驻逻辑). 详见 [sysd.md](sysd.md).

- `sysu`: systemd 风格的状态定义 / 打印输出. 详见 [sysu.md](sysu.md).

- `pa` / `p_w` / `p_u`: 平台抽象及 Windows / Linux 实现. 详见 [win.md](win.md).

---

`unsafe` 代码的说明见 [unsafe.md](unsafe.md).

## 项目约定

关于编写代码的具体要求, 详见 `AGENTS.md` (`代码约定` 章节),
以及源代码各模块中的注释, 此处不再重复. 因为本项目主要使用 AI 写代码,
人类主要只做架构设计和代码审核.

- 依赖 `pm-bin` 提供 CLI 脚手架 (与 pm-tc 一致).

### 代码 (文档) 格式化

```sh
cargo fmt  # rust 代码
deno fmt  # 其余文件 (比如 markdown)
```

## 构建与测试

详见 `AGENTS.md` (`常用命令` 章节), 以及 CI (`.github/workflows/ci.yml`).
