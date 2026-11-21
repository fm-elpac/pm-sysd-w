// 日志写入压力测试的公共工具.
//
// 本文件只提供小工具: 进程运行, 数据库读取 (node:sqlite), 内存采样,
// 配置生成, 数字格式化. 场景定义在 scenarios.js, 运行逻辑在 stress.js.

"use strict";

const fs = require("fs");
const path = require("path");
const childProcess = require("child_process");
const { DatabaseSync } = require("node:sqlite");

const REPO_ROOT = path.resolve(__dirname, "..", "..");
const EMIT_JS = path.join(__dirname, "emit.js");

// 服务输出结束后, 等待日志写入线程排空一段时间, 再优雅退出 init.
const DRAIN_MS = 1500;

// 内存采样默认间隔 (毫秒). Linux 直接读 /proc, 采样开销很小.
const DEFAULT_SAMPLE_MS = 200;

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

// 运行一个短命令, 返回 { code, stdout, stderr }.
function run(cmd, args, opts = {}) {
  return new Promise((resolve) => {
    const child = childProcess.spawn(cmd, args, {
      cwd: opts.cwd,
      env: opts.env,
      stdio: ["ignore", "pipe", "pipe"],
      windowsHide: true,
    });
    let stdout = "";
    let stderr = "";
    child.stdout.on("data", (d) => {
      stdout += d;
    });
    child.stderr.on("data", (d) => {
      stderr += d;
    });
    child.on("error", (e) => {
      resolve({ code: -1, stdout, stderr: String(e) });
    });
    child.on("close", (code) => {
      resolve({ code, stdout, stderr });
    });
  });
}

// 运行一个长驻进程, 返回 { child, pid, stdout, stderr }.
function spawnLong(cmd, args, opts = {}) {
  const child = childProcess.spawn(cmd, args, {
    cwd: opts.cwd,
    env: opts.env,
    stdio: ["ignore", "pipe", "pipe"],
    windowsHide: true,
  });
  let stdout = "";
  let stderr = "";
  child.stdout.on("data", (d) => {
    stdout += d;
  });
  child.stderr.on("data", (d) => {
    stderr += d;
  });
  return { child, pid: child.pid, stdout: () => stdout, stderr: () => stderr };
}

// 查找已编译的二进制, 优先 release.
function findBinary(name) {
  const exe = process.platform === "win32" ? ".exe" : "";
  for (
    const dir of [
      path.join(REPO_ROOT, "target", "release"),
      path.join(REPO_ROOT, "target", "debug"),
    ]
  ) {
    const p = path.join(dir, name + exe);
    if (fs.existsSync(p)) {
      return p;
    }
  }
  return null;
}

// 把路径转换为正斜杠形式. pm-tc 的 ExecStart 按空白切分且不支持引号,
// 因此命令与参数中不能出现空白或反斜杠; 正斜杠在 Windows 上同样可用.
function toPosix(p) {
  return p.split(path.sep).join("/");
}

// 生成 toml 基本字符串 (转义反斜杠与双引号).
function tomlStr(s) {
  return '"' + String(s).replace(/\\/g, "\\\\").replace(/"/g, '\\"') + '"';
}

// 把覆盖环境变量对象转换为可传给 child_process 的 env 对象.
//
// 若外部进程环境中已存在同名环境变量, 则优先使用外部值 (方便外部覆盖脚本
// 内置值), 并输出一行警告.
function buildEnv(overrides) {
  const env = { ...process.env };
  for (const [k, v] of Object.entries(overrides || {})) {
    if (Object.prototype.hasOwnProperty.call(process.env, k)) {
      console.warn(
        `warning: env ${k} already set (${
          process.env[k]
        }), using external value instead of ${v}`,
      );
      env[k] = process.env[k];
    } else {
      env[k] = String(v);
    }
  }
  return env;
}

// 生成一个 service 的 ExecStart 命令 argv.
// mode: full (全速) / rate (限速). rate 额外携带 rateBytesPerSec.
function execArgv(scenario, i, dir) {
  const marker = toPosix(path.join(dir, `done-${i}`));
  const argv = [
    "node",
    toPosix(EMIT_JS),
    scenario.mode,
    String(scenario.durationSec * 1000),
    String(scenario.lineBytes),
    `${scenario.name}-${i}`,
    marker,
  ];
  if (scenario.mode === "rate") {
    argv.push(String(scenario.rateBytesPerSec));
  }
  return argv;
}

// 生成一个场景的集中式配置, 返回 { confPath, dbPath }.
// opts.noIndex 为 true 时, 在 [env] 中写入 SYSD_DB_LOG_NO_INDEX = "1".
function buildConfig(scenario, dir, opts = {}) {
  const lines = [];
  lines.push("pm_tc = 1");
  lines.push("");
  lines.push("[env]");
  lines.push("SYSD_DB = " + tomlStr("stress.db"));
  if (opts.noIndex) {
    lines.push("SYSD_DB_LOG_NO_INDEX = " + tomlStr("1"));
  }
  lines.push("");

  for (let i = 0; i < scenario.services; i++) {
    const name = `stress-${i}.service`;
    const label = `${scenario.name}-${i}`;
    const exec = execArgv(scenario, i, dir).join(" ");
    lines.push("[[t.sysd]]");
    lines.push(tomlStr("env:Unit") + " = " + tomlStr(name));
    lines.push(
      tomlStr("env:Unit.Description") + " = " + tomlStr(`log stress ${label}`),
    );
    lines.push(tomlStr("env:Service.ExecStart") + " = " + tomlStr(exec));
    lines.push("");
  }

  const confPath = path.join(dir, "log-stress.conf.toml");
  fs.writeFileSync(confPath, lines.join("\n"));
  return { confPath, dbPath: path.join(dir, "stress.db") };
}

// 等待 init 就绪: 反复用 whoami 连接, 直到成功或超时.
async function waitReady(ctl, machine, dir, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const r = await run(ctl, ["--machine=" + machine, "whoami", "--json"], {
      cwd: dir,
    });
    if (r.code === 0) {
      return true;
    }
    await sleep(50);
  }
  return false;
}

// 读取一个 service 的 marker 文件.
function readMarker(dir, i) {
  const p = path.join(dir, `done-${i}`);
  if (!fs.existsSync(p)) {
    return null;
  }
  const text = fs.readFileSync(p, "utf8").trim();
  const parts = text.split("\n").map((s) => Number(s.trim()));
  return {
    outCount: Number.isFinite(parts[0]) ? parts[0] : 0,
    errCount: Number.isFinite(parts[1]) ? parts[1] : 0,
    outBytes: Number.isFinite(parts[2]) ? parts[2] : 0,
    errBytes: Number.isFinite(parts[3]) ? parts[3] : 0,
  };
}

// 汇总场景内所有 service 的产出, 返回 { lines, bytes, missing }.
function readProduced(scenario, dir) {
  let lines = 0;
  let bytes = 0;
  let missing = false;
  for (let i = 0; i < scenario.services; i++) {
    const m = readMarker(dir, i);
    if (!m) {
      missing = true;
      continue;
    }
    lines += m.outCount + m.errCount;
    bytes += m.outBytes + m.errBytes;
  }
  return { lines, bytes, missing };
}

// 用 node:sqlite 读取数据库统计. bytes 为 true 时额外统计 msg 总长度.
// 测试数据为 ASCII (pad 为 `x`), 因此 SQLite `LENGTH(msg)` 字符数等于字节数.
// 打开失败时重试几次, 避免 init 退出后仍有短暂占用.
async function readDbStats(dbPath, opts = {}) {
  let lastErr = null;
  for (let i = 0; i < 5; i++) {
    let db = null;
    try {
      db = new DatabaseSync(dbPath, { readOnly: true });
      const countRow = db
        .prepare(
          "SELECT COUNT(*) AS n FROM log WHERE level IN ('stdout','stderr')",
        )
        .get();
      const totalRow = db.prepare("SELECT COUNT(*) AS n FROM log").get();
      let bytesRow = null;
      if (opts.bytes) {
        bytesRow = db
          .prepare(
            "SELECT COALESCE(SUM(LENGTH(msg)), 0) AS b FROM log WHERE level IN ('stdout','stderr')",
          )
          .get();
      }
      db.close();
      return {
        stored: Number(countRow.n),
        totalRows: Number(totalRow.n),
        storedBytes: bytesRow ? Number(bytesRow.b) : null,
      };
    } catch (e) {
      lastErr = e;
      if (db) {
        try {
          db.close();
        } catch (_) {
          // 忽略关闭失败, 继续重试.
        }
      }
      await sleep(200);
    }
  }
  return {
    stored: null,
    totalRows: null,
    storedBytes: null,
    error: lastErr ? String(lastErr) : "read db failed",
  };
}

// 读取指定进程的内存占用 (RSS, 单位字节). 不支持时返回 null.
function readRss(pid) {
  if (process.platform === "linux") {
    const p = `/proc/${pid}/status`;
    if (!fs.existsSync(p)) {
      return null;
    }
    const text = fs.readFileSync(p, "utf8");
    const m = text.match(/^VmRSS:\s+(\d+)\s+kB$/m);
    return m ? Number(m[1]) * 1024 : null;
  }
  if (process.platform === "win32") {
    try {
      const out = childProcess.execFileSync(
        "powershell",
        [
          "-NoProfile",
          "-NonInteractive",
          "-Command",
          `(Get-Process -Id ${pid} -ErrorAction SilentlyContinue).WorkingSet64`,
        ],
        {
          encoding: "utf8",
          windowsHide: true,
          stdio: ["ignore", "pipe", "pipe"],
        },
      );
      const text = String(out).trim();
      // 进程已退出时 stdout 为空, 应视为无数据 (而不是 0).
      if (text === "") {
        return null;
      }
      const n = Number(text);
      return Number.isFinite(n) ? n : null;
    } catch (_) {
      return null;
    }
  }
  return null;
}

// 启动内存采样, 返回 { stop }.
// stop() 返回 { min, max, avg, count, samples } (均为字节或样本).
function startRssSampler(pid, intervalMs) {
  let min = null;
  let max = null;
  let sum = 0;
  let count = 0;
  const samples = [];
  const timer = setInterval(() => {
    const v = readRss(pid);
    if (v === null) {
      return;
    }
    samples.push(v);
    sum += v;
    count++;
    if (min === null || v < min) {
      min = v;
    }
    if (max === null || v > max) {
      max = v;
    }
  }, intervalMs);
  return {
    stop() {
      clearInterval(timer);
      return {
        min,
        max,
        avg: count > 0 ? sum / count : null,
        count,
        samples,
      };
    },
  };
}

// 统计 init stderr 中丢弃警告出现的次数 (限速警告, 只反映是否发生丢弃).
function countDrops(stderr) {
  const re = /dropping log/g;
  const m = stderr.match(re);
  return m ? m.length : 0;
}

function fmt(n) {
  if (n === null || n === undefined) {
    return "-";
  }
  if (typeof n === "number") {
    return Math.round(n).toLocaleString("en-US");
  }
  return String(n);
}

function fmtBytes(n) {
  if (n === null || n === undefined) {
    return "-";
  }
  const units = ["B", "KB", "MB", "GB"];
  let v = n;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v.toFixed(1)}${units[i]}`;
}

// 格式化吞吐, 同时输出 条/s 与字节/s (MB/s 等).
function fmtThroughput(linesPerSec, bytesPerSec) {
  const lines = linesPerSec === null || linesPerSec === undefined
    ? "-"
    : `${fmt(linesPerSec)} 条/s`;
  const bytes = bytesPerSec === null || bytesPerSec === undefined
    ? "-"
    : `${fmtBytes(bytesPerSec)}/s`;
  return `${lines} (${bytes})`;
}

module.exports = {
  REPO_ROOT,
  EMIT_JS,
  DRAIN_MS,
  DEFAULT_SAMPLE_MS,
  sleep,
  run,
  spawnLong,
  findBinary,
  toPosix,
  tomlStr,
  buildEnv,
  buildConfig,
  waitReady,
  readMarker,
  readProduced,
  readDbStats,
  readRss,
  startRssSampler,
  countDrops,
  fmt,
  fmtBytes,
  fmtThroughput,
};
