# 日志模块 (logd)

`logd` 是日志核心逻辑 (类似 journald, 简化实现). 日志存入 sqlite 数据库,
数据库写入由 `db` 模块的日志批发线程与日志写入线程协作完成.

`logd` 负责: 日志写入 (含运行中进程 stdout/stderr 采集), 日志查询 (分页),
日志自动清理. 防御性限制见 [limit.md](limit.md).

- 对外句柄可 clone, 写入为无阻塞投递.

- 日志批发线程持有一个内部缓冲区, 同时检查待写总条数
  (`SYSD_LIMIT_LOG_PENDING_N`) 与总大小 (`SYSD_LIMIT_LOG_PENDING_BYTE`),
  任一达到限制时丢弃日志并限速警告. 日志写入线程每次批发一批日志, 在单个 sqlite
  事务内批量写入.

- 日志输入 MPSC 通道容量 (`SYSD_LIMIT_LOG_CHANNEL_CAPACITY_N`)
  限制通道内堆积的消息数量, 避免批发线程来不及处理时无限堆积.
  待写日志数据最大占用约 2 倍限制的内存: 批发线程内部缓冲,
  加上已发给日志写入线程待写入的一批数据.

- 子进程 stdout/stderr 由后台任务按行读取, 流式解码 (Linux UTF-8, Windows 支持
  ACP 回退).

- 日志查询为分页接口, 保证单页不超过单条消息长度限制.

---

具体实现细节见 `logd` 与 `db` 模块的代码注释.

压力测试与参数实验结果见 [log_test.md](log_test.md).
