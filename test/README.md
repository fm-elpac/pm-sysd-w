# 测试脚本 test/

本目录存放手动运行的测试脚本 (与 Rust 单元测试不同, 单元测试代码放在各自
`test.rs` 中).

## test-permission.ps1

Windows 命名管道权限测试脚本 (PowerShell, 需管理员与 Windows).

运行:

```powershell
powershell -ExecutionPolicy Bypass -File test\test-permission.ps1
```

## log/

日志写入压力测试脚本, 包含默认全速输出, 缓冲调优, 超长日志, 限速搜索四类测试.
说明见 [log/README.md](log/README.md).

运行:

```sh
cargo build --release --features cli_init
cargo build --release --features cli_ctl

node test/log/stress.js
```

## log-encode-bench.js

日志消息 JSON 编码长度实测脚本, 用于推导 `logd` 估算日志编码长度的 a, b 系数.

运行:

```sh
node test/log-encode-bench.js
```
