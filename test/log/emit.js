// 日志压力测试的子进程脚本 (由 service 的 ExecStart 启动).
//
// 用法:
//   node emit.js full  DURATION_MS LINE_BYTES LABEL MARKER
//   node emit.js rate  DURATION_MS LINE_BYTES LABEL MARKER RATE_BYTES_PER_SEC
//
// full: 在指定时长内向 stdout 与 stderr 各持续全速输出.
// rate: 同样输出, 但把两者合计的负载字节速率限制在 RATE_BYTES_PER_SEC.
//
// MARKER 文件在结束后写入 4 行: stdout 条数, stderr 条数, stdout 负载字节,
// stderr 负载字节 (负载字节不含行尾换行). 测试脚本据此统计实际产出.

"use strict";

const fs = require("fs");
const { once } = require("events");

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function byteLen(s) {
  return Buffer.byteLength(s, "utf8");
}

function endStream(stream) {
  return new Promise((resolve, reject) => {
    if (stream.writableEnded || stream.destroyed) {
      resolve();
      return;
    }
    const onError = (e) => reject(e);
    stream.once("error", onError);
    stream.end(() => {
      stream.removeListener("error", onError);
      resolve();
    });
  });
}

async function writeLine(stream, line) {
  if (!stream.write(line + "\n")) {
    await once(stream, "drain");
  }
}

async function runFull(durationMs, lineBytes, label, marker) {
  const pad = "x".repeat(Math.max(0, lineBytes));
  const deadline = Date.now() + durationMs;
  let outCount = 0;
  let errCount = 0;
  let outBytes = 0;
  let errBytes = 0;

  while (Date.now() < deadline) {
    const outLine = label + " " + outCount + " " + pad;
    outBytes += byteLen(outLine);
    await writeLine(process.stdout, outLine);
    outCount++;

    const errLine = label + " e" + errCount + " " + pad;
    errBytes += byteLen(errLine);
    await writeLine(process.stderr, errLine);
    errCount++;
  }

  await Promise.all([endStream(process.stdout), endStream(process.stderr)]);
  fs.writeFileSync(
    marker,
    `${outCount}\n${errCount}\n${outBytes}\n${errBytes}\n`,
  );
}

async function runRate(durationMs, lineBytes, label, marker, rateBytesPerSec) {
  const pad = "x".repeat(Math.max(0, lineBytes));
  const deadline = Date.now() + durationMs;
  let outCount = 0;
  let errCount = 0;
  let outBytes = 0;
  let errBytes = 0;
  let budget = 0;
  let last = Date.now();

  while (Date.now() < deadline) {
    const now = Date.now();
    budget += ((now - last) * rateBytesPerSec) / 1000;
    last = now;

    let wrote = false;

    const outLine = label + " " + outCount + " " + pad;
    const outCost = byteLen(outLine);
    if (budget >= outCost) {
      budget -= outCost;
      await writeLine(process.stdout, outLine);
      outBytes += outCost;
      outCount++;
      wrote = true;
    }

    const errLine = label + " e" + errCount + " " + pad;
    const errCost = byteLen(errLine);
    if (budget >= errCost) {
      budget -= errCost;
      await writeLine(process.stderr, errLine);
      errBytes += errCost;
      errCount++;
      wrote = true;
    }

    if (!wrote) {
      await sleep(1);
    }
  }

  await Promise.all([endStream(process.stdout), endStream(process.stderr)]);
  fs.writeFileSync(
    marker,
    `${outCount}\n${errCount}\n${outBytes}\n${errBytes}\n`,
  );
}

async function main() {
  const mode = process.argv[2];
  const durationMs = Number(process.argv[3]);
  const lineBytes = Number(process.argv[4]);
  const label = process.argv[5];
  const marker = process.argv[6];

  if (mode === "full") {
    await runFull(durationMs, lineBytes, label, marker);
  } else if (mode === "rate") {
    const rateBytesPerSec = Number(process.argv[7]);
    await runRate(durationMs, lineBytes, label, marker, rateBytesPerSec);
  } else {
    throw new Error(`unknown mode: ${mode}`);
  }
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
