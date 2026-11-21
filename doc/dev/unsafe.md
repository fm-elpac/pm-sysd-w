# unsafe 代码说明

整个 crate 禁用 `unsafe` (`#![deny(unsafe_code)]`). 例外是少数无安全替代的
Windows API, 集中在少数模块内以 `#[allow(unsafe_code)]` 覆盖, 把 `unsafe`
调用封装为 safe 接口.

例外模块:

- `p_w::console`: Windows 控制台自适应.
- `p_w::acl`: 命名管道 DACL 设置.
- `p_w::acp`: 系统代码页 (ACP) 获取.
- `p_w::service::u`: Windows 服务 (SCM) 集成.

每个例外模块的具体封装方式与 unsafe 调用点见代码注释.
