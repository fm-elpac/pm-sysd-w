//! 日志查询: 分页读取与日志查询 task.

use tokio::sync::{mpsc, oneshot};

use crate::{
    db::{DbClient, LogEntry},
    jsonl::{SdReqLogRO, SdRes, SdResLogEntry as JsonLogEntry, encode_frame},
    pa::MAX_MESSAGE_LEN,
};

use super::{Logd, LogdError};

/// 日志分页结果
#[derive(Debug, Clone)]
pub struct LogPage {
    /// 匹配条件的日志总量
    pub total: u64,
    /// 下一页游标 (无更多日志时为空)
    pub cursor: Option<i64>,
    /// 本页日志列表 (按 id 升序)
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

    /// 分页读取日志 (可选按 level / unit 过滤)
    ///
    /// `cursor` 为 `Some(id)` 时读取 `id < cursor` 的日志; 为 `None` 时读取最新.
    /// 返回: 总量 / 下一页游标 / 本页日志 (按 id 升序).
    pub async fn page_entries(
        &self,
        level: Option<&str>,
        unit: Option<&str>,
        limit: u32,
        cursor: Option<i64>,
    ) -> Result<LogPage, LogdError> {
        page_entries_with(&self.db, level, unit, limit, cursor).await
    }
}

/// 分页读取日志 (供日志查询 task 与 [`Logd::page_entries`] 使用)
async fn page_entries_with(
    db: &DbClient,
    level: Option<&str>,
    unit: Option<&str>,
    limit: u32,
    cursor: Option<i64>,
) -> Result<LogPage, LogdError> {
    let total = db
        .log_total(level, unit)
        .await
        .map_err(|e| LogdError::Db(e.to_string()))?;

    // 多取 1 条, 用于判断是否还有下一页
    let fetch = limit.saturating_add(1).max(1);
    let raw = db
        .log_page_raw(level, unit, fetch, cursor, MAX_MESSAGE_LEN)
        .await
        .map_err(|e| LogdError::Db(e.to_string()))?;
    let mut rows = raw.entries;

    let has_more = raw.size_limited || rows.len() as u64 > limit as u64;
    if rows.len() as u64 > limit as u64 {
        rows.truncate(limit as usize);
    }

    // 当前页按 id 倒序取到; 记录本页最旧 id 作为下一页游标.
    let next_cursor = if has_more {
        rows.last().map(|e| e.id)
    } else {
        None
    };
    rows.reverse();

    Ok(LogPage {
        total,
        cursor: next_cursor,
        entries: rows,
    })
}

/// 处理日志查询请求, 返回响应
async fn log_ro_resp(db: &DbClient, cmd: &SdReqLogRO) -> SdRes {
    match cmd {
        SdReqLogRO::Query(query) => {
            let page = match page_entries_with(
                db,
                query.level.as_deref(),
                query.unit.as_deref(),
                query.limit,
                query.cursor,
            )
            .await
            {
                Ok(page) => page,
                Err(e) => return SdRes::Err(e.to_string()),
            };

            let mut logs: Vec<JsonLogEntry> = page
                .entries
                .into_iter()
                .map(|e| JsonLogEntry {
                    id: e.id,
                    ts: e.ts,
                    level: e.level,
                    unit: e.unit,
                    msg: e.msg,
                })
                .collect();
            // 最新在前, 便于按前缀选择要返回的日志.
            logs.reverse();

            let total = page.total;
            let original_len = logs.len();
            let selected = select_logs_by_estimate(&logs, MAX_MESSAGE_LEN);
            logs.truncate(selected);

            // 估算截断时, 游标指向本页最旧一条 (被截断的更旧日志下次继续拉取);
            // 未截断时保持原游标.
            let mut cursor = if selected < original_len {
                logs.last().map(|e| e.id)
            } else {
                page.cursor
            };

            // 兜底: encode 实测一次; 若仍超限且不止 1 条, 从末尾 (最旧) 移除.
            loop {
                let resp = SdRes::Logs {
                    total,
                    cursor,
                    logs: logs.clone(),
                };
                let line = match encode_frame(&resp) {
                    Ok(line) => line,
                    Err(e) => return SdRes::Err(e.to_string()),
                };
                if logs.len() <= 1 || line.len() <= MAX_MESSAGE_LEN {
                    return resp;
                }
                logs.pop();
                cursor = logs.last().map(|e| e.id);
            }
        }
    }
}

/// 日志查询 task 主循环
pub(super) async fn run(
    mut rx: mpsc::Receiver<(SdReqLogRO, oneshot::Sender<SdRes>)>,
    db: DbClient,
) {
    while let Some((cmd, reply)) = rx.recv().await {
        reply.send(log_ro_resp(&db, &cmd).await).unwrap();
    }
}

/// JSON 编码放大系数 (根据 test/log-encode-bench.js 实测, 含 30% 余量)
const LOG_JSON_EXPAND: f64 = 7.8;

/// 每条日志 JSON 固定开销 (同上, 单位: 字节)
const LOG_JSON_OVERHEAD: f64 = 137.0;

/// 估算单条日志 JSON 编码后的字节数
fn estimate_log_entry_len(e: &JsonLogEntry) -> usize {
    (e.msg.len() as f64 * LOG_JSON_EXPAND
        + e.ts.len() as f64
        + e.level.len() as f64
        + e.unit.as_ref().map_or(0.0, |u| u.len() as f64)
        + LOG_JSON_OVERHEAD)
        .ceil() as usize
}

/// 从日志列表 (最新在前) 选择前缀, 使估算总长度不超过 `max`.
///
/// 至少返回 1 条 (若列表非空).
fn select_logs_by_estimate(logs: &[JsonLogEntry], max: usize) -> usize {
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
