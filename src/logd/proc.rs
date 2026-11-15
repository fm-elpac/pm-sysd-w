//! 子进程 stdout/stderr 采集.

use std::str;

use log::warn;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, BufReader};

use super::{Logd, MAX_LOG_LEN};

impl Logd {
    /// 启动后台任务: 按行读取子进程的 stdout/stderr, 写入对应 unit 日志.
    ///
    /// `level` 分别为 `stdout` / `stderr`.
    pub fn spawn_log_reader<R>(&self, name: &str, level: &str, reader: R)
    where
        R: AsyncRead + Unpin + Send + 'static,
    {
        let logd = self.clone();
        let name = name.to_string();
        let level = level.to_string();
        tokio::spawn(async move {
            let mut reader = BufReader::new(reader);
            loop {
                let (mut buf, ended) = match read_log_chunk(&mut reader, MAX_LOG_LEN).await {
                    // 读到 EOF: 管道关闭 (子进程退出), 结束本任务.
                    Ok(Some(v)) => v,
                    Ok(None) => break,
                    Err(e) => {
                        warn!("read `{}` {}: {}", name, level, e);
                        break;
                    }
                };
                if ended {
                    // 去掉行尾换行符 (Windows `\r\n` / Unix `\n`)
                    while buf.last() == Some(&b'\r') {
                        buf.pop();
                    }
                }
                match str::from_utf8(&buf) {
                    // 超长行由 logd 自动拆分为多条日志
                    Ok(s) => {
                        logd.write_with(&level, Some(&name), s)
                            .expect("write subprocess output log");
                    }
                    // 非 UTF-8 输出: 换成一条 error 日志, 写入对应 unit.
                    Err(_) => {
                        logd.write_with(
                            "error",
                            Some(&name),
                            &format!(
                                "subprocess {} output is not utf-8; replaced with this error",
                                level
                            ),
                        )
                        .expect("write subprocess non-utf-8 log");
                    }
                }
            }
        });
    }
}

/// 从 subprocess 输出中读取一段日志内容.
///
/// 最多读取 `max` 字节: 到换行符为止, 或填满 `max` 字节 (超长行拆成多段).
/// 返回 `(内容, 是否以换行结束)`; EOF 时返回 `None`.
pub(super) async fn read_log_chunk<R>(
    reader: &mut R,
    max: usize,
) -> std::io::Result<Option<(Vec<u8>, bool)>>
where
    R: AsyncBufRead + Unpin,
{
    let mut buf: Vec<u8> = Vec::with_capacity(max);
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            if buf.is_empty() {
                return Ok(None);
            }
            // EOF: 残留内容作为最后一段.
            return Ok(Some((buf, false)));
        }

        let rem = max - buf.len();
        if let Some(pos) = available.iter().position(|&b| b == b'\n') {
            if pos <= rem {
                buf.extend_from_slice(&available[..pos]);
                reader.consume(pos + 1);
                return Ok(Some((buf, true)));
            }
            // 换行在剩余容量之后: 先填满一段, 下一段再处理换行.
            buf.extend_from_slice(&available[..rem]);
            reader.consume(rem);
            return Ok(Some((buf, false)));
        }

        let take = available.len().min(rem);
        buf.extend_from_slice(&available[..take]);
        reader.consume(take);
        if buf.len() == max {
            return Ok(Some((buf, false)));
        }
    }
}
