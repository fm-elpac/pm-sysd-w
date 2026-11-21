# Windows 集成 (pa / p_w / p_u)

本页记录平台抽象与 Windows 集成相关内容. `unsafe` 代码见 [unsafe.md](unsafe.md).

## 平台抽象 (pa)

`pa` 是底层平台抽象接口, 使用条件编译选择实现: Windows 用 `p_w` (主要平台),
Linux 用 `p_u` (仅用于测试). 其余模块只依赖 `pa`.

抽象两部分差异: 进程管理 (启动 / 停止 / 读取输出), 与 IPC 连接 (命名管道 / UNIX
socket).

## Windows 实现 (p_w)

`p_w` 只包含平台相关代码, 不做抽象 / 封装. IPC 使用命名管道, 只允许本机连接,
禁止网络连接.

## Linux 实现 (p_u)

`p_u` 仅用于方便测试. IPC 使用当前目录的 UNIX socket.

## 命名管道与权限

命名管道名称由 `pa::Instance` 决定. `--system` 实例额外创建只读管道,
供普通用户进行只读查询. 具体名称与 ACL 见 `pa` / `p_w::acl` 代码注释.

命名管道权限的手动验证脚本为 `test/test-permission.ps1` (需管理员,
自动化测试不覆盖跨用户权限).

## 安装脚本与 Windows 服务

`install/setup.ps1` 是实验性的安装 / 卸载脚本 (per-user 服务模板),
行为见用户文档与脚本本身. `pm-sysd-init` 的 Windows 服务 (SCM) 集成见
`p_w::service` 代码注释.

## 已知限制

安装脚本把 exe 放在当前用户目录, 但服务模板是机器级 (HKLM) 配置,
因此目前仅适合单用户场景.
