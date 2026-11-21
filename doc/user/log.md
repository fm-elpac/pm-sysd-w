# pm-sysd-log 命令与日志说明

`pm-sysd-log` 查看日志 (类似 `journalctl`).

未指定实例时, 优先尝试 `--system`, 若系统实例未运行则回退到 `--user`. 连接
`--system` 时, 普通用户 (非管理员) 自动改用只读连接.

## 用法

```sh
# 显示最近日志 (跳到末尾, 默认)
pm-sysd-log --user

# 只显示指定 unit 的最近日志 (-eu 组合)
pm-sysd-log --user -eu test-run
```

## 通用选项

```sh
-e  # 跳到日志末尾 (默认)
-u --unit=UNIT
-x --catalog  # 初版无效果

   --system
   --user
-M --machine=NAME

-h --help
   --version
```

- `-u` / `--unit` 的 unit 名称未写后缀时, 自动补 `.service`.

## 日志说明

- 日志仅保留最近 72 小时, 每 1 小时自动清理一次.

- service 进程的 stdout / stderr 会按行写入对应 unit 的日志.
  单行超长时会被拆分为多条日志. 输出不保证 UTF-8, 解码失败时会换成一条 error
  日志.
