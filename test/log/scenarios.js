// 日志写入压力测试的场景定义.
//
// 场景分为 4 类:
// - default: 默认配置全速输出 (含内存监测).
// - tune:    在默认配置基础上调整日志缓冲/内存相关环境变量.
// - long:    超长单行日志 (100KB ~ 1MB), 以字节吞吐为主.
// - rate:    限速输出, 由 stress.js 做二分搜索, 找 0% 丢失的最高速率.

"use strict";

const MiB = 1024 * 1024;

// 默认全速输出场景. metric 为 lines: 用日志条数计算丢弃率.
const defaultScenarios = [
  {
    name: "s1",
    mode: "full",
    metric: "lines",
    services: 1,
    lineBytes: 60,
    durationSec: 20,
    env: {},
  },
  {
    name: "s4",
    mode: "full",
    metric: "lines",
    services: 4,
    lineBytes: 60,
    durationSec: 20,
    env: {},
  },
  {
    name: "s8",
    mode: "full",
    metric: "lines",
    services: 8,
    lineBytes: 60,
    durationSec: 20,
    env: {},
  },
  {
    name: "s16",
    mode: "full",
    metric: "lines",
    services: 16,
    lineBytes: 60,
    durationSec: 20,
    env: {},
  },
  {
    name: "s4-long",
    mode: "full",
    metric: "lines",
    services: 4,
    lineBytes: 1000,
    durationSec: 20,
    env: {},
  },
];

// 调整日志缓冲/内存后的全速输出场景: 只按比例放大待写日志总大小上限,
// 其余日志限制使用默认值 (见 src/conf/limit.rs).
const tuneScenarios = [
  {
    name: "tune-s8-b2",
    mode: "full",
    metric: "lines",
    services: 8,
    lineBytes: 60,
    durationSec: 20,
    env: {
      SYSD_LIMIT_LOG_PENDING_BYTE: 20 * MiB,
    },
  },
  {
    name: "tune-s8-b4",
    mode: "full",
    metric: "lines",
    services: 8,
    lineBytes: 60,
    durationSec: 20,
    env: {
      SYSD_LIMIT_LOG_PENDING_BYTE: 40 * MiB,
    },
  },
  {
    name: "tune-s8-b8",
    mode: "full",
    metric: "lines",
    services: 8,
    lineBytes: 60,
    durationSec: 20,
    env: {
      SYSD_LIMIT_LOG_PENDING_BYTE: 80 * MiB,
    },
  },
];

// 超长单行日志场景. metric 为 bytes: 用负载字节数计算丢弃率.
// lineBytes 为每行填充字节, 实际行长还会加上 label 与计数前缀.
const longScenarios = [
  {
    name: "long-100k",
    mode: "full",
    metric: "bytes",
    services: 1,
    lineBytes: 100 * 1024,
    durationSec: 10,
    env: {},
  },
  {
    name: "long-500k",
    mode: "full",
    metric: "bytes",
    services: 1,
    lineBytes: 500 * 1024,
    durationSec: 10,
    env: {},
  },
  {
    name: "long-1m",
    mode: "full",
    metric: "bytes",
    services: 1,
    lineBytes: 1024 * 1024,
    durationSec: 10,
    env: {},
  },
];

// 限速搜索的基础场景 (1 个 service, 短行). rateBytesPerSec 由 stress.js 填充.
const rateBaseScenario = {
  name: "rate",
  mode: "rate",
  metric: "lines",
  services: 1,
  lineBytes: 60,
  durationSec: 10,
  rateBytesPerSec: 0,
  env: {},
};

module.exports = {
  defaultScenarios,
  tuneScenarios,
  longScenarios,
  rateBaseScenario,
};
