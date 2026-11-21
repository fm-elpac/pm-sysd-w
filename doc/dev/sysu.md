# sysu 模块 (systemd 风格输出)

`sysu` 是 systemd 风格的状态定义 / 打印输出模块, 只负责把内部状态渲染成
`systemctl` / `journalctl` 风格的可读输出与退出码, 不参与业务逻辑.

下级模块:

- `sysu::s`: 状态定义 (尽量与 systemd 保持一致).
- `sysu::ctl`: pm-sysd-ctl 的 systemctl 风格输出与退出码.
- `sysu::log`: pm-sysd-log 的 journalctl 风格输出.
- `sysu::p` (内部): systemd 风格的打印 / 字符串格式化.

## 风格约定

- 只求相似, 不追求与 systemd 完全一致, 不使用 pager.
- 与本项目无关或无法提供的字段会被省略.
- 输出中不出现 `systemd` 等词 (本项目不是 systemd).

字段级省略清单与具体输出格式见代码注释.
