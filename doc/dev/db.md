# 数据库 schema

pm-sysd-init 使用 sqlite 数据库存储持久状态与日志. 数据库格式属于内部 API,
不作为外部稳定 API 提供.

## 文件位置

默认数据库文件名为 `pm_sysd_w_v1.db`, 与主配置文件在同一目录 (主配置文件为
`dir/XXX.toml` 时, 数据库文件为 `dir/pm_sysd_w_v1.db`). 路径可用主配置文件中的
`[env] SYSD_DB` 修改 (默认值 `pm_sysd_w_v1.db`, 相对路径相对主配置文件目录).

## 版本号与升级

数据库内必须存有 schema 版本号 (`pm_sysd_w_meta` 表的 `schema_version` 键,
整数), 用于版本检测与自动升级. 打开流程:

1. 先尝试读取 schema 版本与 `program` 标识; 在确认数据库版本之前,
   不对数据库做任何写入.
2. 只有 `pm_sysd_w_meta` 表不存在时, 才创建数据库 schema (即 新数据库).
3. 只有 当前版本 < 当前程序支持版本 (需要升级) 时, 才执行迁移; 迁移封装在
   单个事务 中, 原子完成 (要么成功, 要么失败, 不会 "升级一半").
4. 打开 更新版本数据库 (由更新版本的程序创建): 报错退出, 不做降级.
5. 迁移完成后 更新 `schema_version` 与相关元信息.

新建数据库时, 写入当前 schema 版本 (`db::SCHEMA_VERSION`), 以及 程序名称 / 版本
/ 时间 等元信息.

当前 `SCHEMA_VERSION = 1`, 尚无迁移步骤. 在 v0.1.0 发布之前 (覆盖所有 `0.1.0-*`
预发布版本) 不升级 schema 版本. 后续每次 schema 变更需: 递增版本号,
并补充对应的迁移代码 (在 `db` 模块中).

## 表结构 (v1)

### `pm_sysd_w_meta`

键值对, 存储元信息 (含 schema 版本). 打开数据库时, 需检查 `program` 是否为
`pm-sysd-w`, 避免与其它程序的数据库混淆; 不一致则报错退出.

| 列      | 类型 | 约束        | 说明 |
| ------- | ---- | ----------- | ---- |
| `key`   | TEXT | PRIMARY KEY | 键   |
| `value` | TEXT | NOT NULL    | 值   |

内置键:

| 键                     | 值示例                     | 说明                                         |
| ---------------------- | -------------------------- | -------------------------------------------- |
| `program`              | `pm-sysd-w`                | 程序名称 (打开时检查)                        |
| `schema_version`       | `1`                        | schema 版本 (整数, 以字符串存储)             |
| `created_by_version`   | `0.1.0-a1`                 | 创建数据库时的 pm-sysd-w 版本 (调试用)       |
| `last_upgrade_version` | `0.1.0-a1`                 | 最后更新 schema 时的 pm-sysd-w 版本 (调试用) |
| `created_at`           | `2026-11-10T00:00:00.000Z` | 数据库创建时间 (调试用)                      |
| `updated_at`           | `2026-11-10T00:00:00.000Z` | schema 最后更新时间 (调试用)                 |

时间均采用 ISO 8601 (UTC, 类似 js 的 `Date.toISOString()` 格式).

### `unit`

unit 的持久状态 (enable / mask 等).

| 列        | 类型    | 约束               | 说明              |
| --------- | ------- | ------------------ | ----------------- |
| `name`    | TEXT    | PRIMARY KEY        | unit 名称         |
| `enabled` | INTEGER | NOT NULL DEFAULT 0 | 是否 enable (0/1) |
| `masked`  | INTEGER | NOT NULL DEFAULT 0 | 是否 mask (0/1)   |

### `log`

日志记录 (由 `logd` 模块写入).

| 列      | 类型    | 约束                      | 说明                                                                                  |
| ------- | ------- | ------------------------- | ------------------------------------------------------------------------------------- |
| `id`    | INTEGER | PRIMARY KEY AUTOINCREMENT | 自增 id                                                                               |
| `ts`    | TEXT    | NOT NULL                  | 时间戳 (UTC, ISO 8601)                                                                |
| `level` | TEXT    | NOT NULL                  | 日志级别, 用于过滤 (如 `debug`/`info`/`warn`/`error`, 子进程输出为 `stdout`/`stderr`) |
| `unit`  | TEXT    |                           | 关联 unit 名 (可为空)                                                                 |
| `msg`   | TEXT    | NOT NULL                  | 日志内容                                                                              |

### 索引

| 索引           | 表    | 列     | 说明                      |
| -------------- | ----- | ------ | ------------------------- |
| `idx_log_unit` | `log` | `unit` | 按 unit 过滤日志 (查询)   |
| `idx_log_ts`   | `log` | `ts`   | 日志自动清理 / 按时间查询 |

## 访问方式

数据库读写统一通过 `db` 模块封装, 其它模块不直接操作 sqlite. `db` 模块内部 启动
2 个专用 (同步) 线程: 数据库读写线程 与 日志写入线程; 只有这两个线程直接 访问
sqlite. tokio 的 async 线程通过 MPSC 向数据库读写线程发送请求, 请求中携带 响应
MPSC, 读写线程完成后回传. 详见 `src/db/`.
