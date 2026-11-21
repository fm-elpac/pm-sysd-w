//! 日志查询: 分页读取与日志查询 task.

use tokio::sync::{mpsc, oneshot};

use crate::{
    db::{DbClient, LogEntry, LogPos},
    jsonl::{SdReqLogRO, SdReqPagePos, SdRes, SdResLogItem, SdResLogPage, encode_frame},
    sysd::send_reply,
};

use super::{Logd, LogdError};

/// 日志分页结果
#[derive(Debug, Clone)]
pub struct LogPage {
    /// 匹配条件的日志总量
    pub total: u64,
    /// 下一页游标 (无更多日志时为空)
    ///
    /// 游标方向与本次查询方向一致: 升序查询时下一页为 `id > cursor`,
    /// 降序查询时下一页为 `id < cursor`.
    pub cursor: Option<i64>,
    /// 本页日志列表 (按请求方向排序)
    pub entries: Vec<LogEntry>,
}

impl Logd {
    /// 发送日志查询请求 (等待投递完成, 不等待处理/回执)
    pub async fn req_log_ro(
        &self,
        cmd: SdReqLogRO,
        reply: oneshot::Sender<SdRes>,
    ) -> Result<(), mpsc::error::SendError<(SdReqLogRO, oneshot::Sender<SdRes>)>> {
        self.tx.send((cmd, reply)).await
    }

    /// 分页读取日志 (可选按 level / unit 过滤).
    ///
    /// `asc` 决定返回方向 (`true` 升序 / `false` 降序), `pos` 决定开始位置.
    /// 返回: 总量 / 下一页游标 / 本页日志 (按 `asc` 方向排序).
    pub async fn page_entries(
        &self,
        level: Option<&str>,
        unit: Option<&str>,
        asc: bool,
        pos: SdReqPagePos<i64>,
        limit: u32,
    ) -> Result<LogPage, LogdError> {
        page_entries_raw(&self.db, level, unit, asc, pos, limit, self.ipc_m_byte).await
    }
}

/// 分页读取日志 (返回按请求方向排序的原始页).
async fn page_entries_raw(
    db: &DbClient,
    level: Option<&str>,
    unit: Option<&str>,
    asc: bool,
    pos: SdReqPagePos<i64>,
    limit: u32,
    ipc_m_byte: usize,
) -> Result<LogPage, LogdError> {
    // 多取 1 条, 用于判断是否还有下一页
    let fetch = limit.saturating_add(1).max(1);
    let raw = db
        .log_page(level, unit, asc, to_log_pos(pos), fetch, ipc_m_byte)
        .await
        .map_err(|e| LogdError::Db(e.to_string()))?;
    let total = raw.total;
    let mut rows = raw.entries;

    let has_more = raw.size_limited || rows.len() as u64 > limit as u64;
    if rows.len() as u64 > limit as u64 {
        rows.truncate(limit as usize);
    }

    // 本页最后一条 (在请求方向上) 就是下一页游标.
    let next_cursor = if has_more {
        rows.last().map(|e| e.id)
    } else {
        None
    };

    Ok(LogPage {
        total,
        cursor: next_cursor,
        entries: rows,
    })
}

/// 把 jsonl 的分页位置转换为数据库内部表示.
fn to_log_pos(pos: SdReqPagePos<i64>) -> LogPos {
    match pos {
        SdReqPagePos::Cursor(c) => LogPos::Cursor(c),
        SdReqPagePos::Start(n) => LogPos::Start(n),
        SdReqPagePos::End(n) => LogPos::End(n),
    }
}

/// 处理日志查询请求, 返回响应
async fn log_ro_resp(
    db: &DbClient,
    cmd: &SdReqLogRO,
    ipc_m_byte: usize,
    log_page_n: usize,
) -> SdRes {
    match cmd {
        SdReqLogRO::Query(query) => {
            // 外部请求的一页条数超过硬限制时, 自动按限制截断
            let log_page_n = log_page_n.min(u32::MAX as usize) as u32;
            let limit = query.limit.min(log_page_n);
            let asc = query.asc;
            let pos = query.pos.clone();

            let page = match page_entries_raw(
                db,
                query.filter.level.as_deref(),
                query.filter.unit.as_deref(),
                asc,
                pos,
                limit,
                ipc_m_byte,
            )
            .await
            {
                Ok(page) => page,
                Err(e) => return SdRes::Err(e.to_string()),
            };

            // 数据库已按请求方向返回
            let mut items: Vec<SdResLogItem> = page
                .entries
                .into_iter()
                .map(|e| SdResLogItem {
                    id: e.id,
                    ts: e.ts,
                    level: e.level,
                    unit: e.unit,
                    msg: e.msg,
                })
                .collect();

            let total = page.total;
            let original_len = items.len();
            let selected = select_logs_by_estimate(&items, ipc_m_byte);
            items.truncate(selected);

            // 估算截断时, 游标指向本页保留的最后一条 (被截断的数据下次继续拉取).
            // 未截断时保持原游标.
            let mut cursor = if selected < original_len {
                items.last().map(|e| e.id)
            } else {
                page.cursor
            };

            // 兜底: encode 实测一次. 若仍超限且不止 1 条, 从末尾移除.
            loop {
                let resp = SdRes::Logs(SdResLogPage {
                    total,
                    cursor,
                    list: items.clone(),
                });
                let line = match encode_frame(&resp) {
                    Ok(line) => line,
                    Err(e) => return SdRes::Err(e.to_string()),
                };
                if items.len() <= 1 || line.len() <= ipc_m_byte {
                    return resp;
                }
                items.pop();
                cursor = items.last().map(|e| e.id);
            }
        }
    }
}

/// 日志查询 task 主循环
pub(super) async fn run(
    mut rx: mpsc::Receiver<(SdReqLogRO, oneshot::Sender<SdRes>)>,
    db: DbClient,
    ipc_m_byte: usize,
    log_page_n: usize,
) {
    while let Some((cmd, reply)) = rx.recv().await {
        send_reply(reply, log_ro_resp(&db, &cmd, ipc_m_byte, log_page_n).await);
    }
}

/// JSON 编码放大系数 (根据 test/log-encode-bench.js 实测, 含 30% 余量)
const LOG_JSON_EXPAND: f64 = 7.8;

/// 每条日志 JSON 固定开销 (同上, 单位: 字节)
const LOG_JSON_OVERHEAD: f64 = 137.0;

/// 估算单条日志 JSON 编码后的字节数
fn estimate_log_entry_len(e: &SdResLogItem) -> usize {
    (e.msg.len() as f64 * LOG_JSON_EXPAND
        + e.ts.len() as f64
        + e.level.len() as f64
        + e.unit.as_ref().map_or(0.0, |u| u.len() as f64)
        + LOG_JSON_OVERHEAD)
        .ceil() as usize
}

/// 从日志列表 (按请求方向排序) 选择前缀, 使估算总长度不超过 `max`.
///
/// 若列表非空, 至少返回 1 条.
fn select_logs_by_estimate(logs: &[SdResLogItem], max: usize) -> usize {
    let mut total = 0usize;
    for (i, e) in logs.iter().enumerate() {
        let add = estimate_log_entry_len(e);
        if i > 0 && total.saturating_add(add) > max {
            return i;
        }
        total = total.saturating_add(add);
    }
    logs.len()
}
