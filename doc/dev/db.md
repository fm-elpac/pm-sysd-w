# 数据库文档

pm-sysd-init 使用 sqlite 数据库存储持久状态与日志. 数据库格式属于内部 API,
不作为外部稳定 API 提供.

数据库文件位置与 `SYSD_DB` 配置见用户文档 (`doc/user/config.md`). 数据库访问方式
(专用线程 / MPSC) 见开发文档 (`doc/dev/README.md`).

## 版本号与升级

数据库内必须存有 schema 版本号 (`pm_sysd_w_meta` 表的 `schema_version` 键,
整数), 用于版本检测与自动升级.

打开流程 (版本检查, 初始化, 升级, 全部封装在单个事务中):

1. 打开 sqlite 连接并开始一个事务, 后续步骤都在该事务内完成.

2. 检查 `pm_sysd_w_meta` 表是否存在:

   - 不存在: 视为新数据库, 建表 / 建索引 / 写入初始元数据.

   - 存在: 读取 `program` 与 `schema_version` 并校验:

     - `program` 不是 `pm-sysd-w`: 报错退出.
     - 当前版本 > 支持版本: 报错退出, 不做降级.
     - 当前版本 < 支持版本: 执行迁移, 并更新 `schema_version` 与相关元信息.

3. 提交事务 (任一步失败则整体回滚, 不会 "初始化/升级一半").

---

新建数据库时, 写入当前 schema 版本 (`db::SCHEMA_VERSION`), 以及程序名称, 版本,
时间, 等元数据.

当前 `SCHEMA_VERSION = 1`, 尚无迁移步骤. 在 v0.1.0 发布之前 (覆盖所有 `0.1.0-*`
预发布版本) 不升级 schema 版本.

后续每次 schema 变更需: 递增版本号, 并补充对应的迁移代码 (在 `db` 模块中).

## 表结构 (v1)

### `pm_sysd_w_meta`

键值对, 存储元信息 (含 schema 版本). 打开数据库时, 需检查 `program` 是否为
`pm-sysd-w`, 避免与其它程序的数据库混淆. 不一致则报错退出.

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

db 模块只提供两个 unit 读写 API:

- 读: 一次性读取 unit 表全部数据 (返回 `UnitState` 列表).
- 写: 写入单个 unit 的完整状态 (`UnitState`, 含 `name` / `enabled` / `masked`).

持久状态的清理: 写入数据全部为默认值 (enabled=false 且 masked=false) 时,
直接删除此数据, 避免此表无限膨胀.

### `log`

日志记录 (由 `logd` 模块写入).

| 列      | 类型    | 约束                      | 说明                   |
| ------- | ------- | ------------------------- | ---------------------- |
| `id`    | INTEGER | PRIMARY KEY AUTOINCREMENT | 自增 id                |
| `ts`    | TEXT    | NOT NULL                  | 时间戳 (UTC, ISO 8601) |
| `level` | TEXT    | NOT NULL                  | 日志级别               |
| `unit`  | TEXT    |                           | 关联 unit 名 (可为空)  |
| `msg`   | TEXT    | NOT NULL                  | 日志内容               |

- 日志级别: 用于过滤, 如 `debug`/`info`/`warn`/`error`, subprocess 输出为
  `stdout`/`stderr`.

### 索引

| 索引            | 表    | 列          | 说明                                   |
| --------------- | ----- | ----------- | -------------------------------------- |
| `idx_log_unit`  | `log` | `unit, id`  | 按 unit 过滤日志 (查询, 并按 id 排序)  |
| `idx_log_level` | `log` | `level, id` | 按 level 过滤日志 (查询, 并按 id 排序) |
| `idx_log_ts`    | `log` | `ts`        | 日志自动清理 / 按时间查询              |
