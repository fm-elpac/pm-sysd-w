// 日志写入压力测试主脚本.
//
// 编译 release 二进制后运行 (脚本优先使用 target/release):
//
//   cargo build --release --features cli_init
//   cargo build --release --features cli_ctl
//
// 运行:
//
//   node test/log/stress.js
//
// 选项:
//
//   --type=default|tune|long|rate|all   选择测试类型 (默认 default)
//   --duration=N       覆盖 default/tune/long 场景的持续秒数
//   --scenario=A,B     只运行指定场景 (default/tune/long)
//   --rate-duration=N  限速搜索中每次运行的秒数 (默认 10)
//   --rate-iters=N     限速二分搜索最大迭代次数 (默认 12)
//   --sample-ms=N      内存采样间隔 (默认 200)
//   --no-index         测试全程关闭数据库日志表索引 (SYSD_DB_LOG_NO_INDEX=1)
//   --keep             保留临时目录用于排查
//
// 依赖: node.js (自带 node:sqlite). 不再依赖 sqlite3 命令行工具.

"use strict";

const fs = require("fs");
const path = require("path");

const lib = require("./lib");
const {
  REPO_ROOT,
  DRAIN_MS,
  DEFAULT_SAMPLE_MS,
  sleep,
  run,
  spawnLong,
  findBinary,
  buildEnv,
  buildConfig,
  waitReady,
  readProduced,
  readDbStats,
  startRssSampler,
  countDrops,
  fmt,
  fmtBytes,
  fmtThroughput,
} = lib;

const {
  defaultScenarios,
  tuneScenarios,
  longScenarios,
  rateBaseScenario,
} = require("./scenarios");

const TMP_BASE = path.join(REPO_ROOT, "tmp");
fs.mkdirSync(TMP_BASE, { recursive: true });
const TMP_ROOT = fs.mkdtempSync(path.join(TMP_BASE, "log-stress-"));

let scenarioIndex = 0;
function nextIndex() {
  return scenarioIndex++;
}

function parseArgs(argv) {
  const opts = {
    type: "default",
    duration: null,
    scenario: null,
    keep: false,
    rateDuration: 10,
    rateIters: 12,
    sampleMs: DEFAULT_SAMPLE_MS,
    noIndex: false,
  };
  for (const a of argv) {
    if (a === "--keep") {
      opts.keep = true;
    } else if (a === "--no-index") {
      opts.noIndex = true;
    } else if (a.startsWith("--type=")) {
      opts.type = a.slice("--type=".length);
    } else if (a.startsWith("--duration=")) {
      opts.duration = Number(a.slice("--duration=".length));
    } else if (a.startsWith("--scenario=")) {
      opts.scenario = a.slice("--scenario=".length).split(",");
    } else if (a.startsWith("--rate-duration=")) {
      opts.rateDuration = Number(a.slice("--rate-duration=".length));
    } else if (a.startsWith("--rate-iters=")) {
      opts.rateIters = Number(a.slice("--rate-iters=".length));
    } else if (a.startsWith("--sample-ms=")) {
      opts.sampleMs = Number(a.slice("--sample-ms=".length));
    } else {
      console.error(`unknown argument: ${a}`);
      process.exit(2);
    }
  }

  if (!["default", "tune", "long", "rate", "all"].includes(opts.type)) {
    console.error(
      `--type must be one of: default, tune, long, rate, all (got ${opts.type})`,
    );
    process.exit(2);
  }

  return opts;
}

function scenarioList(type) {
  if (type === "default") {
    return defaultScenarios;
  }
  if (type === "tune") {
    return tuneScenarios;
  }
  if (type === "long") {
    return longScenarios;
  }
  return [];
}

function applyDurationAndFilter(list, opts) {
  return list
    .map((s) => ({
      ...s,
      durationSec: opts.duration !== null ? opts.duration : s.durationSec,
    }))
    .filter((s) => !opts.scenario || opts.scenario.includes(s.name));
}

// 执行一个场景, 返回结果对象.
async function runScenario(scenario, index, bins, opts = {}) {
  const dir = fs.mkdtempSync(path.join(TMP_ROOT, `${scenario.name}-`));
  const machine = `ls${Date.now().toString(36)}${index}`;
  const { confPath, dbPath } = buildConfig(scenario, dir, {
    noIndex: opts.noIndex,
  });
  const env = buildEnv(scenario.env);

  console.log(`[开始] ${scenario.name}: ${new Date().toISOString()}`);

  const init = spawnLong(
    bins.init,
    ["--machine=" + machine, "--conf=" + confPath],
    { cwd: dir, env },
  );

  if (!(await waitReady(bins.ctl, machine, dir, 30000))) {
    init.child.kill("SIGKILL");
    console.log(`[结束] ${scenario.name}: ${new Date().toISOString()}`);
    return {
      name: scenario.name,
      error: "init not ready within 30s",
      initStderr: init.stderr(),
      initStdout: init.stdout(),
      initCode: null,
    };
  }

  const sampleMemory = opts.sampleMemory !== false;
  const sampler = sampleMemory
    ? startRssSampler(init.pid, opts.sampleMs || DEFAULT_SAMPLE_MS)
    : null;

  const t0 = Date.now();

  const starts = [];
  for (let i = 0; i < scenario.services; i++) {
    starts.push(
      run(
        bins.ctl,
        ["--machine=" + machine, "start", `stress-${i}.service`],
        { cwd: dir },
      ),
    );
  }
  const startResults = await Promise.all(starts);
  const startFailures = startResults.filter((r) => r.code !== 0);

  let t1 = null;
  let markersMissing = false;
  {
    const deadline = Date.now() + scenario.durationSec * 1000 + 60000;
    while (Date.now() < deadline) {
      let all = true;
      for (let i = 0; i < scenario.services; i++) {
        if (!fs.existsSync(path.join(dir, `done-${i}`))) {
          all = false;
          break;
        }
      }
      if (all) {
        t1 = Date.now();
        break;
      }
      await sleep(20);
    }
    if (t1 === null) {
      markersMissing = true;
      t1 = Date.now();
    }
  }

  const elapsedMs = Math.max(1, t1 - t0);
  const produced = readProduced(scenario, dir);
  await sleep(DRAIN_MS);

  const exitRes = await run(bins.ctl, ["--machine=" + machine, "exit"], {
    cwd: dir,
  });

  let initCode = null;
  {
    const deadline = Date.now() + 120000;
    while (Date.now() < deadline) {
      if (init.child.exitCode !== null || init.child.signalCode !== null) {
        initCode = init.child.exitCode;
        break;
      }
      await sleep(50);
    }
    if (
      initCode === null &&
      init.child.exitCode === null &&
      init.child.signalCode === null
    ) {
      init.child.kill("SIGKILL");
      initCode = "timeout-killed";
    } else if (initCode === null) {
      initCode = init.child.exitCode;
    }
  }

  const rss = sampler ? sampler.stop() : null;
  const initStderr = init.stderr();
  const dropWarnings = countDrops(initStderr);

  const db = await readDbStats(dbPath, { bytes: true });
  let dbBytes = null;
  if (fs.existsSync(dbPath)) {
    dbBytes = fs.statSync(dbPath).size;
  }

  const producedN = produced.lines;
  const storedN = db.stored === null ? 0 : db.stored;
  const producedBytes = produced.bytes;
  const storedBytes = db.storedBytes === null ? null : db.storedBytes;

  let dropped;
  let dropPct;
  if (scenario.metric === "bytes") {
    dropped = Math.max(
      0,
      producedBytes - (storedBytes === null ? 0 : storedBytes),
    );
    dropPct = producedBytes > 0 ? (dropped / producedBytes) * 100 : 0;
  } else {
    dropped = Math.max(0, producedN - storedN);
    dropPct = producedN > 0 ? (dropped / producedN) * 100 : 0;
  }

  const droppedBytes = Math.max(
    0,
    producedBytes - (storedBytes === null ? 0 : storedBytes),
  );
  const dropBytesPct = producedBytes > 0
    ? (droppedBytes / producedBytes) * 100
    : 0;

  console.log(`[结束] ${scenario.name}: ${new Date().toISOString()}`);

  return {
    name: scenario.name,
    mode: scenario.mode,
    metric: scenario.metric,
    services: scenario.services,
    durationSec: scenario.durationSec,
    lineBytes: scenario.lineBytes,
    produced: producedN,
    stored: storedN,
    dropped,
    dropPct,
    producedBytes,
    storedBytes,
    droppedBytes,
    dropBytesPct,
    splitFactor: producedN > 0 ? storedN / producedN : null,
    elapsedMs,
    producedPerSec: (producedN * 1000) / elapsedMs,
    storedPerSec: (storedN * 1000) / elapsedMs,
    producedBytesPerSec: (producedBytes * 1000) / elapsedMs,
    storedBytesPerSec: storedBytes === null
      ? null
      : (storedBytes * 1000) / elapsedMs,
    rssMin: rss ? rss.min : null,
    rssMax: rss ? rss.max : null,
    rssAvg: rss ? rss.avg : null,
    rssSamples: rss ? rss.count : 0,
    dbTotalRows: db.totalRows,
    dbBytes,
    dropWarnings,
    startFailures: startFailures.length,
    markersMissing,
    exitCode: exitRes.code,
    initCode,
    initStdout: init.stdout(),
    initStderr,
    dbError: db.error,
  };
}

function isLossless(r) {
  return (
    !r.error &&
    r.startFailures === 0 &&
    !r.markersMissing &&
    r.dropped === 0
  );
}

// 打印限速搜索中的一次尝试详情. targetBytesPerSec 为 null 表示全速摸底,
// 此时不打印目标速率, 用 produced 字节速率表示实测全速.
function printRateAttempt(tag, targetBytesPerSec, r) {
  console.log(`  -- ${tag} --`);
  if (r.error) {
    console.log(`    ERROR: ${r.error}`);
    return;
  }
  if (targetBytesPerSec !== null && targetBytesPerSec !== undefined) {
    console.log(`    target rate : ${fmtBytes(targetBytesPerSec)}/s`);
  }
  console.log(
    `    produced    : ${fmt(r.produced)} lines (${fmtBytes(r.producedBytes)})`,
  );
  console.log(`    stored      : ${fmt(r.stored)} lines`);
  console.log(
    `    dropped     : ${fmt(r.dropped)} lines (${r.dropPct.toFixed(1)}%)`,
  );
  console.log(
    `    produced/s  : ${
      fmtThroughput(r.producedPerSec, r.producedBytesPerSec)
    }`,
  );
  console.log(
    `    stored/s    : ${fmtThroughput(r.storedPerSec, r.storedBytesPerSec)}`,
  );
  console.log(`    lossless    : ${isLossless(r)}`);
  if (r.rssMin !== null && r.rssMin !== undefined) {
    console.log(
      `    rss         : min ${fmtBytes(r.rssMin)}  max ${
        fmtBytes(r.rssMax)
      }  avg ${fmtBytes(r.rssAvg)}  samples ${r.rssSamples}`,
    );
  }
  console.log(`    db rows     : ${fmt(r.dbTotalRows)}`);
  console.log(`    db size     : ${fmtBytes(r.dbBytes)}`);
}

// 限速搜索: 二分查找 0% 丢失的最高日志输出速率 (字节/秒).
// 每次尝试 (全速摸底, 二分扫描, 确认运行) 都打印详细参数与结果.
async function runRateSearch(bins, opts) {
  const duration = opts.rateDuration || 10;
  const iters = opts.rateIters || 12;

  console.log(`  -- rate search params --`);
  console.log(
    `    duration: ${duration}s, iters: ${iters}, services: ${rateBaseScenario.services}, lineBytes: ${rateBaseScenario.lineBytes}`,
  );
  console.log();

  const fullProbe = {
    name: "rate-full",
    mode: "full",
    metric: "lines",
    services: 1,
    lineBytes: 60,
    durationSec: duration,
    env: {},
  };
  const fullRes = await runScenario(fullProbe, nextIndex(), bins, {
    sampleMemory: true,
    sampleMs: opts.sampleMs,
    noIndex: opts.noIndex,
  });
  if (fullRes.error) {
    printRateAttempt("full", null, fullRes);
    return { error: fullRes.error };
  }
  const fullRate = fullRes.producedBytesPerSec;
  if (!(fullRate > 0)) {
    printRateAttempt("full", null, fullRes);
    return { error: "rate-full produced 0 bytes/s" };
  }
  printRateAttempt("full", null, fullRes);
  console.log(`    full speed  : ${fmtBytes(fullRate)}/s`);
  console.log();

  const base = { ...rateBaseScenario, durationSec: duration };
  let low = Math.max(1, fullRate / 1000);
  let high = fullRate;
  let best = low;
  let bestRes = null;
  let runs = 0;

  for (let i = 0; i < iters; i++) {
    const mid = (low + high) / 2;
    const sc = { ...base, name: "rate-scan", rateBytesPerSec: mid };
    const r = await runScenario(sc, nextIndex(), bins, {
      sampleMemory: true,
      sampleMs: opts.sampleMs,
      noIndex: opts.noIndex,
    });
    runs++;
    if (r.error) {
      printRateAttempt(`scan-${runs}`, mid, r);
      return { error: `rate scan error: ${r.error}`, runs };
    }
    const lossless = isLossless(r);
    printRateAttempt(`scan-${runs}`, mid, r);
    console.log(
      `    range       : [${fmtBytes(low)}/s, ${fmtBytes(high)}/s] -> ${
        lossless ? "lossless, raise low" : "lossy, lower high"
      }`,
    );
    console.log();
    if (lossless) {
      low = mid;
      best = mid;
      bestRes = r;
    } else {
      high = mid;
    }
    if (high - low < fullRate * 0.02) {
      break;
    }
  }

  let verifyRes = null;
  if (bestRes) {
    const sc = { ...base, name: "rate-verify", rateBytesPerSec: best };
    verifyRes = await runScenario(sc, nextIndex(), bins, {
      sampleMemory: true,
      sampleMs: opts.sampleMs,
      noIndex: opts.noIndex,
    });
    runs++;
    printRateAttempt("verify", best, verifyRes);
    console.log();
  }

  return {
    fullRate,
    rateBytesPerSec: best,
    verifyLossless: verifyRes ? isLossless(verifyRes) : null,
    runs,
    fullRes,
    bestRes,
    verifyRes,
    base,
  };
}

function fmtMetric(v, metric) {
  if (metric === "bytes") {
    return fmtBytes(v);
  }
  return fmt(v);
}

function printRow(r) {
  console.log(`--- scenario ${r.name} ---`);
  if (r.error) {
    console.log(`ERROR: ${r.error}`);
    if (r.initStderr) {
      console.log(`init stderr:\n${r.initStderr}`);
    }
    return;
  }
  console.log(`mode          : ${r.mode}`);
  console.log(`services      : ${r.services}`);
  console.log(`duration sec  : ${r.durationSec}`);
  console.log(`lineBytes     : ${r.lineBytes}`);
  console.log(`metric        : ${r.metric}`);
  const prod = r.metric === "bytes" ? r.producedBytes : r.produced;
  const store = r.metric === "bytes" ? r.storedBytes : r.stored;
  const drop = r.metric === "bytes" ? r.droppedBytes : r.dropped;
  console.log(`produced      : ${fmtMetric(prod, r.metric)} (${r.metric})`);
  console.log(`stored        : ${fmtMetric(store, r.metric)} (${r.metric})`);
  console.log(
    `dropped       : ${fmtMetric(drop, r.metric)} (${r.dropPct.toFixed(1)}%)`,
  );
  console.log(
    `produced/s    : ${fmtThroughput(r.producedPerSec, r.producedBytesPerSec)}`,
  );
  console.log(
    `stored/s      : ${fmtThroughput(r.storedPerSec, r.storedBytesPerSec)}`,
  );
  if (r.metric === "bytes" && r.splitFactor !== null) {
    console.log(`split factor  : ${r.splitFactor.toFixed(3)} rows/line`);
  }
  console.log(
    `rss           : min ${fmtBytes(r.rssMin)}  max ${
      fmtBytes(r.rssMax)
    }  avg ${fmtBytes(r.rssAvg)}  samples ${r.rssSamples}`,
  );
  console.log(`db total rows : ${fmt(r.dbTotalRows)}`);
  console.log(`db size       : ${fmtBytes(r.dbBytes)}`);
  console.log(`elapsed ms    : ${fmt(r.elapsedMs)}`);
  console.log(`drop warnings : ${r.dropWarnings}`);
  console.log(`start failures: ${r.startFailures}`);
  console.log(`markers miss  : ${r.markersMissing}`);
  console.log(`ctl exit code : ${r.exitCode}`);
  console.log(`init exit code: ${r.initCode}`);
  if (r.dbError) {
    console.log(`db error      : ${r.dbError}`);
  }
}

function printRateSearch(rr) {
  console.log(`--- rate limit search ---`);
  if (rr.error) {
    console.log(`ERROR: ${rr.error}`);
    return;
  }
  console.log(
    `full speed      : ${
      fmtThroughput(rr.fullRes.producedPerSec, rr.fullRes.producedBytesPerSec)
    }`,
  );
  console.log(`max lossless    : ${fmtBytes(rr.rateBytesPerSec)}/s`);
  console.log(
    `verify lossless : ${
      rr.verifyLossless === null ? "skipped" : rr.verifyLossless
    }`,
  );
  console.log(`runs            : ${rr.runs}`);
  if (rr.bestRes) {
    console.log(
      `at max lossless : produced ${fmt(rr.bestRes.produced)} lines (${
        fmtThroughput(rr.bestRes.producedPerSec, rr.bestRes.producedBytesPerSec)
      }), stored ${fmt(rr.bestRes.stored)} (${
        fmtThroughput(rr.bestRes.storedPerSec, rr.bestRes.storedBytesPerSec)
      }), drop ${rr.bestRes.dropPct.toFixed(1)}%`,
    );
  }
}

function printSummary(results) {
  if (results.length === 0) {
    return;
  }
  console.log();
  console.log("汇总 (summary):");
  console.log(
    [
      "scenario".padEnd(11),
      "services".padStart(8),
      "sec".padStart(4),
      "lineB".padStart(8),
      "metric".padStart(6),
      "produced".padStart(12),
      "stored".padStart(10),
      "dropped".padStart(10),
      "drop%".padStart(7),
      "lines/s".padStart(10),
      "MB/s".padStart(10),
      "rssMax".padStart(10),
      "db".padStart(8),
    ].join(" "),
  );
  for (const r of results) {
    if (r.error) {
      console.log(`${r.name.padEnd(11)} ERROR: ${r.error}`);
      continue;
    }
    const prod = r.metric === "bytes" ? r.producedBytes : r.produced;
    const store = r.metric === "bytes" ? r.storedBytes : r.stored;
    const drop = r.metric === "bytes" ? r.droppedBytes : r.dropped;
    console.log(
      [
        r.name.padEnd(11),
        fmt(r.services).padStart(8),
        fmt(r.durationSec).padStart(4),
        fmt(r.lineBytes).padStart(8),
        r.metric.padStart(6),
        fmtMetric(prod, r.metric).padStart(12),
        fmtMetric(store, r.metric).padStart(10),
        fmtMetric(drop, r.metric).padStart(10),
        `${r.dropPct.toFixed(1)}%`.padStart(7),
        fmt(r.storedPerSec).padStart(10),
        fmtBytes(r.storedBytesPerSec).padStart(10),
        fmtBytes(r.rssMax).padStart(10),
        fmtBytes(r.dbBytes).padStart(8),
      ].join(" "),
    );
  }
}

async function main() {
  const opts = parseArgs(process.argv.slice(2));

  const bins = {
    init: findBinary("pm-sysd-init"),
    ctl: findBinary("pm-sysd-ctl"),
  };
  if (!bins.init || !bins.ctl) {
    console.error("未找到 pm-sysd-init / pm-sysd-ctl, 请先编译:");
    console.error("  cargo build --release --features cli_init");
    console.error("  cargo build --release --features cli_ctl");
    process.exit(1);
  }

  console.log(`临时目录: ${TMP_ROOT}`);
  console.log(`init: ${bins.init}`);
  console.log(`ctl:  ${bins.ctl}`);
  console.log(`node: ${process.execPath} (node:sqlite available)`);
  console.log();

  const types = opts.type === "all"
    ? ["default", "tune", "long", "rate"]
    : [opts.type];
  const results = [];

  for (const type of types) {
    if (type === "rate") {
      const rr = await runRateSearch(bins, opts);
      printRateSearch(rr);
      continue;
    }
    const list = scenarioList(type);
    const scenarios = applyDurationAndFilter(list, opts);
    for (const sc of scenarios) {
      const r = await runScenario(sc, nextIndex(), bins, {
        sampleMemory: true,
        sampleMs: opts.sampleMs,
        noIndex: opts.noIndex,
      });
      results.push(r);
      printRow(r);
    }
  }

  printSummary(results);

  if (!opts.keep) {
    fs.rmSync(TMP_ROOT, { recursive: true, force: true });
    console.log(`已删除临时目录: ${TMP_ROOT}`);
  } else {
    console.log(`保留临时目录 (--keep): ${TMP_ROOT}`);
  }
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
