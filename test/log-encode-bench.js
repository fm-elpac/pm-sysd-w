// 日志消息 JSON 编码长度实测脚本.
//
// 按照 jsonl 模块的格式构造单条日志响应, 用 JSON.stringify 实测编码长度.
// 根据实测结果, 留 30% 余量, 计算 logd 估算日志编码长度所用的 a, b 系数:
//
//   estimated = ceil(msg_utf8_len * a + ts_len + level_len + unit_len + b)
//
// 其中 ts_len / level_len / unit_len 为对应字段的 UTF-8 字节长度 (unit 为 null
// 时按 0 计). 本脚本用 unit = null 构造样本, 因此 b 已包含 `unit:null` 语法开销,
// 不会与实际代码中额外计入的 unit_len 冲突.
//
// 运行: node test/log-encode-bench.js
"use strict";

const byteLen = (s) => Buffer.byteLength(s, "utf8");

// 与 jsonl::SdRes::Logs 一致: 外层 `res` + `data`, 内部 logs 数组.
function encodeLogsResponse(msg) {
  const entry = {
    id: 1,
    ts: "2026-11-14T00:00:00.000Z",
    level: "info",
    unit: null,
    msg,
  };
  const resp = {
    res: "logs",
    data: {
      total: 1,
      cursor: null,
      logs: [entry],
    },
  };
  return JSON.stringify(resp);
}

function measure(msg) {
  const json = encodeLogsResponse(msg);
  const y = byteLen(json);
  const x = byteLen(msg);
  // 脚本固定 ts / level, unit 为 null; 代码中会再按实际字段长度计入 ts / level /
  // unit, 因此这里从实测总长中扣除这些内容长度, 剩余部分由 a*x + b 覆盖.
  const tsLen = byteLen("2026-11-14T00:00:00.000Z");
  const levelLen = byteLen("info");
  const fixed = tsLen + levelLen;
  const z = y - fixed;
  return { x, y, z };
}

function makeSamples() {
  const samples = [];
  const lengths = [0, 1, 10, 100, 1000, 10000, 100000, 500000];

  for (const n of lengths) {
    samples.push({ name: `ascii-${n}`, msg: "a".repeat(n) });
    samples.push({ name: `cjk-${n}`, msg: "猫".repeat(n) });
    samples.push({ name: `escape-${n}`, msg: '"\\\n\r\t'.repeat(n) });
    samples.push({ name: `control-${n}`, msg: "\u0000\u001b\u001f".repeat(n) });
    samples.push({
      name: `mixed-${n}`,
      msg: 'abc猫"\\\n\u0000def'.repeat(Math.max(1, Math.floor(n / 12))),
    });
  }

  return samples;
}

function main() {
  const samples = makeSamples();
  const rows = samples.map((s) => {
    const m = measure(s.msg);
    return { name: s.name, ...m };
  });

  console.log(
    "实测结果 (x=msg UTF-8 字节, y=单条响应 JSON 字节, z=y-ts-level):",
  );
  for (const r of rows) {
    console.log(
      `${r.name.padEnd(16)} x=${String(r.x).padStart(7)} y=${
        String(r.y).padStart(9)
      } z=${String(r.z).padStart(8)}`,
    );
  }

  const empty = rows.find((r) => r.name === "ascii-0");
  const b = Math.ceil(empty.z * 1.3);
  let a = 0;
  for (const r of rows) {
    if (r.x === 0) continue;
    const slope = (r.z * 1.3 - b) / r.x;
    a = Math.max(a, slope);
  }

  console.log();
  console.log("30% 余量后的建议系数:");
  console.log(`  LOG_JSON_EXPAND   (a) = ${a.toFixed(6)}`);
  console.log(`  LOG_JSON_OVERHEAD (b) = ${b}`);
}

main();
