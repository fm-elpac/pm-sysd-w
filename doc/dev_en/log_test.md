# Log Stress Test (log_test)

This document records log-write stress tests and parameter experiments. The test
scripts live in `test/log/`; requirements and design are in [log.md](log.md),
and defensive limits are in [limit.md](limit.md).

## Test method

The stress script `test/log/stress.js` generates a temp config with several
services, starts `pm-sysd-init --machine` using the release binary, starts all
services concurrently, lets them output to stdout/stderr continuously, then
compares each service's actual output with the rows stored in SQLite, observing
log writes, drops, memory usage, and throughput. Script usage, scenario
definitions, options, and result interpretation are in
[test/log/README.md](../../test/log/README.md) and are not repeated here.

The baseline test uses the script's default options (default config and default
values) and runs the full four-type suite, i.e. `--type=all`: default
full-speed, buffer tuning, long single-line, and rate-limit search. First build
the release binaries:

```sh
cargo build --release --features cli_init
cargo build --release --features cli_ctl

node test/log/stress.js --type=all
```

Linux uses the default sampling interval `--sample-ms=200`; on Windows each
sample starts a powershell process, so `--sample-ms=1000` is used instead.

## Baseline test (2026-11-21)

This section is the full-suite run with default options/default values. The two
machines ran one after the other (not simultaneously), both without errors. The
code is at commit `3befc39`, with the log defaults already raised by commit
`625aafc`: `SYSD_LIMIT_LOG_PENDING_BYTE` 10MB, `SYSD_LIMIT_LOG_PENDING_N`
100000, `SYSD_LIMIT_LOG_CHANNEL_CAPACITY_N` 1000.

### Test environment

| Item   | Local (Linux)                        | win10 test machine                           |
| ------ | ------------------------------------ | -------------------------------------------- |
| System | Arch Linux, kernel 7.3.5, x86_64     | Windows 10 IoT Enterprise LTSC 19044, 64-bit |
| CPU    | AMD Ryzen 7 5825U, 8 cores 8 threads | AMD Ryzen 7 5825U, 4 cores 4 threads         |
| RAM    | 16 GB                                | 8 GB                                         |
| Disk   | 200 GB virtual disk, btrfs (ssd)     | 100 GB virtual disk, C: NTFS                 |
| node   | v26.13.1                             | v26.13.0                                     |
| rustc  | 1.100.0                              | 1.100.0                                      |

### Results

Produced = the sum of each service's stdout/stderr lines (or bytes); stored =
SQLite rows with `level IN ('stdout','stderr')` (or msg bytes); dropped =
produced - stored. Throughput is recorded in both lines/s and MB/s, and the
`pm-sysd-init` process RSS is sampled.

#### Default full-speed (60B lines, s4-long 1000B, 20s per scenario)

| Platform | Scenario | Produced   | Stored    | Drop% | Stored throughput (lines/s / MB/s) | RSS max |
| -------- | -------- | ---------- | --------- | ----- | ---------------------------------- | ------- |
| Linux    | s1       | 7,003,862  | 3,723,077 | 46.8  | 185,699 / 13.0                     | 85.1MB  |
| Linux    | s4       | 26,057,162 | 2,843,088 | 89.1  | 141,736 / 9.9                      | 104.6MB |
| Linux    | s8       | 33,629,254 | 1,669,311 | 95.0  | 83,063 / 5.8                       | 114.4MB |
| Linux    | s16      | 34,107,322 | 1,148,532 | 96.6  | 56,698 / 4.0                       | 115.8MB |
| Linux    | s4-long  | 13,072,424 | 1,230,309 | 90.6  | 61,344 / 59.5                      | 72.4MB  |
| win10    | s1       | 2,040,092  | 1,202,578 | 41.1  | 59,747 / 4.1                       | 60.3MB  |
| win10    | s4       | 10,235,900 | 687,229   | 93.3  | 33,582 / 2.3                       | 64.8MB  |
| win10    | s8       | 12,797,468 | 595,510   | 95.3  | 27,547 / 1.9                       | 60.9MB  |
| win10    | s16      | 13,049,344 | 537,286   | 95.9  | 23,938 / 1.7                       | 65.7MB  |
| win10    | s4-long  | 5,007,814  | 194,335   | 96.1  | 9,256 / 9.0                        | 38.2MB  |

#### Buffer tuning (s8 full-speed, 20s per scenario)

| Platform | Scenario   | Produced   | Stored    | Drop% | Stored throughput (lines/s / MB/s) | RSS max |
| -------- | ---------- | ---------- | --------- | ----- | ---------------------------------- | ------- |
| Linux    | tune-s8-b2 | 33,368,948 | 1,930,662 | 94.2  | 96,096 / 7.4                       | 121.5MB |
| Linux    | tune-s8-b4 | 34,193,910 | 1,840,724 | 94.6  | 91,624 / 7.1                       | 118.3MB |
| Linux    | tune-s8-b8 | 33,131,536 | 1,688,009 | 94.9  | 83,914 / 6.5                       | 117.4MB |
| win10    | tune-s8-b2 | 12,349,978 | 531,112   | 95.7  | 24,866 / 1.9                       | 67.4MB  |
| win10    | tune-s8-b4 | 12,225,436 | 629,143   | 94.9  | 29,504 / 2.3                       | 71.8MB  |
| win10    | tune-s8-b8 | 11,929,248 | 641,521   | 94.6  | 30,120 / 2.3                       | 73.0MB  |

#### Long single-line (one service, 10s per scenario, counted in bytes)

| Platform | Scenario  | Produced | Stored | Drop% | Stored throughput (lines/s / MB/s) | Split factor | RSS max |
| -------- | --------- | -------- | ------ | ----- | ---------------------------------- | ------------ | ------- |
| Linux    | long-100k | 7.3GB    | 1.9GB  | 73.7  | 2,009 / 196.3                      | 0.263        | 55.0MB  |
| Linux    | long-500k | 6.5GB    | 2.1GB  | 67.7  | 1,799 / 214.6                      | 1.323        | 48.5MB  |
| Linux    | long-1m   | 5.6GB    | 2.1GB  | 62.3  | 775 / 216.8                        | 1.348        | 50.4MB  |
| win10    | long-100k | 2.9GB    | 1.8GB  | 37.8  | 1,861 / 181.8                      | 0.622        | 33.2MB  |
| win10    | long-500k | 1.5GB    | 1.5GB  | 0.1   | 617 / 150.6                        | 1.999        | 24.8MB  |
| win10    | long-1m   | 1.5GB    | 1.5GB  | 0.0   | 431 / 143.6                        | 3.000        | 21.8MB  |

#### Rate-limit search (one service, short lines, 10s per attempt)

| Platform | Attempt | Target   | Produced  | Stored    | Drop% | Stored throughput (lines/s / MB/s) | RSS max |
| -------- | ------- | -------- | --------- | --------- | ----- | ---------------------------------- | ------- |
| Linux    | full    | -        | 3,533,512 | 1,864,922 | 47.2  | 185,491 / 14.1                     | 77.0MB  |
| Linux    | scan-1  | 13.4MB/s | 1,769,709 | 1,769,709 | 0.0   | 176,003 / 13.3                     | 35.8MB  |
| Linux    | scan-2  | 20.1MB/s | 2,642,583 | 1,941,799 | 26.5  | 193,099 / 14.7                     | 81.8MB  |
| Linux    | scan-3  | 16.7MB/s | 2,207,907 | 2,059,065 | 6.7   | 204,984 / 15.5                     | 76.2MB  |
| Linux    | scan-4  | 15.1MB/s | 1,990,109 | 1,965,275 | 1.2   | 195,608 / 14.8                     | 77.1MB  |
| Linux    | scan-5  | 14.2MB/s | 1,879,901 | 1,879,901 | 0.0   | 187,111 / 14.2                     | 47.7MB  |
| Linux    | scan-6  | 14.7MB/s | 1,935,002 | 1,935,002 | 0.0   | 192,672 / 14.6                     | 51.3MB  |
| Linux    | verify  | 14.7MB/s | 1,887,578 | 1,887,578 | 0.0   | 187,931 / 14.6                     | 56.1MB  |
| win10    | full    | -        | 1,110,930 | 667,096   | 40.0  | 64,311 / 4.9                       | 57.5MB  |
| win10    | scan-1  | 4.1MB/s  | 537,195   | 537,195   | 0.0   | 52,018 / 3.9                       | 14.9MB  |
| win10    | scan-2  | 6.1MB/s  | 754,693   | 754,144   | 0.1   | 73,019 / 5.5                       | 15.2MB  |
| win10    | scan-3  | 5.1MB/s  | 670,167   | 668,737   | 0.2   | 64,681 / 4.9                       | 18.1MB  |
| win10    | scan-4  | 4.6MB/s  | 603,338   | 603,338   | 0.0   | 58,531 / 4.4                       | 15.0MB  |
| win10    | scan-5  | 4.8MB/s  | 635,925   | 635,925   | 0.0   | 61,662 / 4.7                       | 16.6MB  |
| win10    | scan-6  | 4.9MB/s  | 653,309   | 653,309   | 0.0   | 63,177 / 4.8                       | 14.7MB  |
| win10    | verify  | 4.9MB/s  | 637,494   | 637,494   | 0.0   | 61,695 / 4.8                       | 15.3MB  |

0%-loss ceiling: Linux 14.7MB/s (verify passed), win10 4.9MB/s (verify passed).
Full-speed probe: Linux 26.8MB/s (351,453 lines/s), win10 8.1MB/s (107,098
lines/s).

### Platform comparison

- Lossless rate: Linux 14.7MB/s (about 190k lines/s) is about 3x win10's 4.9MB/s
  (about 63k lines/s).
- Single-service full-speed stored throughput: Linux 185,699 lines/s (13.0MB/s)
  is about 3x win10's 59,747 lines/s (4.1MB/s).
- Full-speed multi-service drop rates are similar in magnitude: Linux
  89.1%-96.6%, win10 93.3%-95.9%.
- Buffer tuning does not clearly help on either platform; the drop rate stays
  close to default s8.
- Long-line stored byte throughput: Linux 196.3-216.8MB/s, slightly above
  win10's 143.6-181.8MB/s.
- Init RSS peak is clearly higher on Linux: default s16 115.8MB and tune b2
  121.5MB, versus win10's s16 65.7MB and tune b8 73.0MB.

### Error analysis

- Each scenario runs once without averaging; short-line high-concurrency and
  rate-limit search results are noisy, so exact values are only for order-of-
  magnitude reference.
- The two machines use different sampling intervals (Linux 200ms, win10 1000ms),
  so win10 has fewer samples and its RSS peak may be missed.
- Test subprocesses and init run on the same machine and compete for CPU and
  disk in concurrent scenarios.
- The two machines differ in hardware (8 cores/16GB vs 4 cores/8GB), so platform
  comparison includes hardware differences, not only software differences.
- The temp dir and database are on disk (not tmpfs), so disk write bandwidth is
  one of the variables.

## Parameter experiments

The experiments below were done on 2026-11-20, when the code defaults were
`SYSD_LIMIT_LOG_PENDING_BYTE` 5MB, `SYSD_LIMIT_LOG_PENDING_N` 10k, and
`SYSD_LIMIT_LOG_CHANNEL_CAPACITY_N` 100. Some were done on top of `--no-index`,
with 10 seconds per scenario (`--duration=10`). The tables follow the original
recording, so some record only one throughput unit (lines/s or MB/s) and memory
(RSS) is only recorded for some experiments. These conclusions were later
adopted by commit `625aafc` and became the current defaults.

### Log table index disabled (no-index)

`--no-index` (`SYSD_DB_LOG_NO_INDEX=1`) makes a newly created database skip the
`idx_log_unit` / `idx_log_level` / `idx_log_ts` indexes. Compared below against
the indexed 10-second baseline from the same session; all are single runs.

#### Linux

default / tune short-line scenarios (stored throughput lines/s / MB/s, drop rate
in parentheses):

| Scenario   | Indexed                | No-index               |
| ---------- | ---------------------- | ---------------------- |
| s1         | 152,005 / 10.6 (57.2%) | 361,483 / 25.1 (2.1%)  |
| s4         | 123,937 / 8.6 (90.5%)  | 341,455 / 23.7 (74.2%) |
| s8         | 74,434 / 5.1 (95.6%)   | 190,408 / 13.2 (88.5%) |
| s16        | 52,119 / 3.7 (97.0%)   | 157,902 / 11.1 (91.2%) |
| s4-long    | 53,879 / 52.3 (91.6%)  | 79,258 / 76.9 (87.5%)  |
| tune-s8-b2 | 88,563 / 6.8 (94.5%)   | 214,642 / 16.5 (86.8%) |
| tune-s8-b4 | 78,990 / 6.0 (94.8%)   | 262,768 / 20.2 (84.1%) |
| tune-s8-b8 | 86,999 / 6.7 (94.7%)   | 278,315 / 21.3 (83.1%) |

Long-line scenarios (counted in bytes):

| Scenario  | Indexed lines/s / MB/s | No-index lines/s / MB/s | Indexed drop% | No-index drop% |
| --------- | ---------------------- | ----------------------- | ------------- | -------------- |
| long-100k | 1,496 / 146.1          | 1,499 / 146.4           | 79.5          | 80.4           |
| long-500k | 1,753 / 175.0          | 1,726 / 184.3           | 74.3          | 72.0           |
| long-1m   | 650 / 173.3            | 608 / 167.6             | 68.8          | 67.1           |

Disabling the indexes cuts the short-line drop rate sharply (s1 from 57.2% to
2.1%), showing index maintenance is the main short-line write bottleneck. At
higher concurrency stored throughput rises about 2-3x. The cost is a larger DB
file (s1 314MB to 466MB, s4-long 749MB to 1.0GB) and full-table scans for
queries by unit/level/ts. Long lines are basically unchanged.

#### Windows

default / tune short-line scenarios (stored throughput lines/s / MB/s, drop rate
in parentheses):

| Scenario   | Indexed               | No-index               |
| ---------- | --------------------- | ---------------------- |
| s1         | 78,148 / 5.4 (5.5%)   | 73,874 / 5.1 (1.6%)    |
| s4         | 54,783 / 3.8 (87.5%)  | 234,877 / 16.2 (42.5%) |
| s8         | 41,300 / 2.8 (92.4%)  | 207,188 / 14.2 (51.0%) |
| s16        | 38,249 / 2.7 (92.8%)  | 137,497 / 9.6 (72.4%)  |
| s4-long    | 14,083 / 13.7 (93.7%) | 72,416 / 70.2 (62.0%)  |
| tune-s8-b2 | 30,494 / 2.3 (94.3%)  | 184,545 / 14.1 (58.2%) |
| tune-s8-b4 | 34,217 / 2.6 (93.5%)  | 221,297 / 16.9 (55.2%) |
| tune-s8-b8 | 30,836 / 2.3 (94.6%)  | 112,232 / 8.6 (78.4%)  |

Long-line scenarios (counted in bytes):

| Scenario  | Indexed lines/s / MB/s | No-index lines/s / MB/s | Indexed drop% | No-index drop% |
| --------- | ---------------------- | ----------------------- | ------------- | -------------- |
| long-100k | 1,369 / 133.7          | 1,669 / 163.0           | 49.5          | 40.4           |
| long-500k | 605 / 144.9            | 565 / 137.9             | 3.6           | 0.1            |
| long-1m   | 408 / 136.1            | 377 / 125.8             | 0.3           | 0.0            |

Disabling the indexes cuts the high-concurrency drop rate sharply (s4 stored
rises from 54,783 to 234,877 lines/s, about 4x; s8 from 41,300 to 207,188, about
5x). Single-service (s1) is already near-lossless and changes little. tune-s8-b8
at 78.4% is higher than its neighbors and is single-run noise. Long lines are
basically unchanged.

### PENDING_N sweep (no-index)

On top of `--no-index`, `SYSD_LIMIT_LOG_PENDING_N` was set to 10k (then
default), 100k, and 1M, with everything else unchanged and 10 seconds per
scenario. For the tune scenarios the external variable overrides the built-in N
(the script prints a warning), so the tune rows below are the external uniform
N. Cells show stored throughput lines/s and drop rate.

#### Linux

default short-line scenarios:

| Scenario | N=10k           | N=100k          | N=1M            |
| -------- | --------------- | --------------- | --------------- |
| s1       | 361,483 / 2.1%  | 342,684 / 3.1%  | 345,313 / 0.3%  |
| s4       | 341,455 / 74.2% | 346,467 / 71.5% | 394,646 / 69.0% |
| s8       | 190,408 / 88.5% | 219,134 / 86.6% | 238,106 / 85.8% |
| s16      | 157,902 / 91.2% | 193,661 / 88.7% | 186,679 / 89.4% |
| s4-long  | 79,258 / 87.5%  | 77,278 / 87.5%  | 90,697 / 85.7%  |

tune short-line scenarios:

| Scenario   | N=10k           | N=100k          | N=1M            |
| ---------- | --------------- | --------------- | --------------- |
| tune-s8-b2 | 214,642 / 86.8% | 277,088 / 82.9% | 248,902 / 84.3% |
| tune-s8-b4 | 262,768 / 84.1% | 275,160 / 83.0% | 297,073 / 81.4% |
| tune-s8-b8 | 278,315 / 83.1% | 294,539 / 81.7% | 319,460 / 80.2% |

Going from 10k to 100k gives the clearest gain (s8 +25% stored, s16 +23%,
tune-s8-b2 +29%). Going from 100k to 1M gives diminishing and unstable returns
while memory grows clearly: tune-s8-b8 RSS max from 107.6MB to 263.1MB,
tune-s8-b4 from 79.0MB to 165.2MB. Long-line scenarios are basically unaffected.
The rate-search lossless rate is noisy and is not compared.

#### Windows

default short-line scenarios:

| Scenario | N=10k           | N=100k          | N=1M            |
| -------- | --------------- | --------------- | --------------- |
| s1       | 73,874 / 1.6%   | 99,700 / 1.5%   | 91,400 / 1.3%   |
| s4       | 234,877 / 42.5% | 231,772 / 41.8% | 245,076 / 36.5% |
| s8       | 207,188 / 51.0% | 193,190 / 58.4% | 166,876 / 66.8% |
| s16      | 137,497 / 72.4% | 123,203 / 74.9% | 130,253 / 73.4% |
| s4-long  | 72,416 / 62.0%  | 63,987 / 68.5%  | 63,773 / 67.5%  |

tune short-line scenarios:

| Scenario   | N=10k           | N=100k          | N=1M            |
| ---------- | --------------- | --------------- | --------------- |
| tune-s8-b2 | 184,545 / 58.2% | 164,775 / 66.4% | 168,279 / 67.0% |
| tune-s8-b4 | 221,297 / 55.2% | 128,617 / 74.2% | 123,376 / 76.2% |
| tune-s8-b8 | 112,232 / 78.4% | 96,369 / 80.9%  | 115,012 / 77.7% |

Unlike Linux, raising PENDING_N on Windows does not help; most scenarios drop
more (s8 rises from 51.0% to 58.4% then 66.8%, and the tune scenarios get worse
overall). Single-run variance is high, so the trend is only qualitative.
Long-line scenarios and the rate-search lossless rate are basically unaffected.

### Log input channel capacity sweep (no-index)

On top of `--no-index`, `SYSD_LIMIT_LOG_CHANNEL_CAPACITY_N` was set to 100 (then
default), 200, 500, and 1000, with everything else unchanged. The full
`--type=all` suite was run and the rate-limit binary search used 10 seconds per
attempt (`--rate-duration=10`). The tune scenarios' built-in values were
overridden uniformly by the external variable (the script prints a warning), so
the tune rows below are the external capacity. Short-line cells show stored
throughput lines/s and drop rate; long-line cells show stored byte throughput
MB/s and drop rate.

#### Linux

default / tune short-line scenarios:

| Scenario   | 100             | 200             | 500             | 1000            |
| ---------- | --------------- | --------------- | --------------- | --------------- |
| s1         | 347,667 / 4.8%  | 376,729 / 0.8%  | 361,908 / 2.3%  | 337,720 / 1.9%  |
| s4         | 326,911 / 75.2% | 337,215 / 74.8% | 299,645 / 77.0% | 308,208 / 76.5% |
| s8         | 192,609 / 88.7% | 189,411 / 88.6% | 197,054 / 88.6% | 182,533 / 89.4% |
| s16        | 154,156 / 91.2% | 145,670 / 91.6% | 143,482 / 92.0% | 129,463 / 92.5% |
| s4-long    | 77,287 / 87.4%  | 90,201 / 86.6%  | 83,827 / 87.2%  | 82,608 / 87.2%  |
| tune-s8-b2 | 213,636 / 87.6% | 215,007 / 86.5% | 195,134 / 88.5% | 198,162 / 88.5% |
| tune-s8-b4 | 226,146 / 85.5% | 269,676 / 84.1% | 233,915 / 86.3% | 229,315 / 86.7% |
| tune-s8-b8 | 254,775 / 84.8% | 321,725 / 81.1% | 253,910 / 84.2% | 258,333 / 84.9% |

Long-line scenarios:

| Scenario  | 100           | 200           | 500           | 1000          |
| --------- | ------------- | ------------- | ------------- | ------------- |
| long-100k | 178.3 / 76.5% | 192.7 / 76.3% | 165.9 / 78.5% | 175.7 / 77.2% |
| long-500k | 190.8 / 72.1% | 206.0 / 70.9% | 184.8 / 71.6% | 181.1 / 71.1% |
| long-1m   | 173.3 / 66.9% | 200.2 / 63.9% | 182.4 / 66.6% | 180.7 / 66.9% |

Rate-limit search 0%-loss ceiling:

| Capacity | Max lossless | Verify |
| -------- | ------------ | ------ |
| 100      | 3.2MB/s      | failed |
| 200      | 2.9MB/s      | passed |
| 500      | 6.6MB/s      | passed |
| 1000     | 13.8MB/s     | passed |

Single-service short lines (s1) benefit the most: the drop rate falls from 4.8%
at 100 to 0.8% at 200, then hovers around 2%. High concurrency and long lines
are basically unaffected. The lossless ceiling rises clearly with capacity,
peaking at 13.8MB/s at 1000. Memory: RSS max barely changes with capacity
(short-line full speed 24-72MB, tune-s8-b8 115-126MB, long lines 31-48MB, rate
limiting 21-25MB), because capacity only adds a few hundred message slots.

#### Windows

default / tune short-line scenarios:

| Scenario   | 100             | 200             | 500             | 1000            |
| ---------- | --------------- | --------------- | --------------- | --------------- |
| s1         | 80,178 / 2.0%   | 80,887 / 0.7%   | 76,584 / 1.0%   | 89,553 / 0.1%   |
| s4         | 232,978 / 43.7% | 204,673 / 46.7% | 176,732 / 46.6% | 220,276 / 47.7% |
| s8         | 207,620 / 56.5% | 178,160 / 65.4% | 169,440 / 67.5% | 171,339 / 67.9% |
| s16        | 103,645 / 81.4% | 105,883 / 79.5% | 124,020 / 76.8% | 128,548 / 77.4% |
| s4-long    | 68,426 / 65.3%  | 61,374 / 69.1%  | 57,696 / 71.0%  | 67,927 / 65.9%  |
| tune-s8-b2 | 188,062 / 58.4% | 207,534 / 58.4% | 218,665 / 53.5% | 195,986 / 56.6% |
| tune-s8-b4 | 172,980 / 62.8% | 129,318 / 72.2% | 130,345 / 72.8% | 92,316 / 79.8%  |
| tune-s8-b8 | 85,277 / 83.4%  | 80,421 / 83.2%  | 83,713 / 83.2%  | 78,554 / 84.1%  |

Long-line scenarios:

| Scenario  | 100           | 200           | 500           | 1000          |
| --------- | ------------- | ------------- | ------------- | ------------- |
| long-100k | 156.3 / 37.5% | 155.1 / 39.5% | 116.7 / 58.5% | 117.7 / 57.4% |
| long-500k | 138.8 / 2.0%  | 88.6 / 12.2%  | 138.8 / 2.0%  | 114.1 / 23.6% |
| long-1m   | 124.9 / 0.0%  | 100.6 / 10.5% | 119.2 / 19.0% | 106.9 / 17.8% |

Rate-limit search 0%-loss ceiling:

| Capacity | Max lossless | Verify |
| -------- | ------------ | ------ |
| 100      | 451.9KB/s    | passed |
| 200      | 917.5KB/s    | passed |
| 500      | 2.8MB/s      | passed |
| 1000     | 5.0MB/s      | failed |

The lossless ceiling rises monotonically with capacity (451.9KB/s to 5.0MB/s,
about 11x), the clearest gain here. Single-service short lines (s1) fall from
2.0% dropped at 100 to 0.1% at 1000, and s16 falls from 81.4% to 77.4%. The
other short-line and long-line scenarios show no stable trend, and single-run
variance is high. The 1000 confirmation run still dropped 0.1%, so the real
lossless threshold is slightly below 5.0MB/s. Memory: RSS max barely changes
with capacity (short-line full speed 14-27MB, tune-s8-b4/b8 35/57MB, long lines
19-26MB, rate limiting 11-15MB).

### SQLite tuning experiment (failed and reverted)

An attempt was made to add `PRAGMA journal_mode = WAL` and
`PRAGMA synchronous = NORMAL` for log writes, expecting to cut per-commit fsync
cost. Re-running the stress test on both machines regressed performance instead:

- Long-line byte throughput roughly halved: Linux 161-176MB/s down to 81-88MB/s,
  Windows 127-144MB/s down to 73-83MB/s.
- Short-line stored throughput dropped about 10-30%.
- Linux lossless rate fell from about 3.6MB/s to about 873KB/s; Windows was
  roughly unchanged (about 477KB/s).

The reason is that this project uses a single shared SQLite connection (one
writer thread, serialized by a mutex), so WAL's read/write concurrency benefit
is unused, while the WAL-then-checkpoint double write adds overhead. fsync was
never the main bottleneck: short lines are limited by per-row INSERT and index
maintenance, and long lines by page write bandwidth. The change was therefore
reverted.

### Key findings

- The short-line write bottleneck is index maintenance; disabling the indexes
  cuts the drop rate sharply.
- Raising `SYSD_LIMIT_LOG_PENDING_N` (10k to 100k) helps on Linux, with
  diminishing returns and more memory beyond that; on Windows it does not help
  and is slightly worse.
- The rate-search lossless ceiling rises with
  `SYSD_LIMIT_LOG_CHANNEL_CAPACITY_N` on both platforms, peaking at 1000, with
  almost no increase in peak memory.
- These findings are the basis for commit `625aafc` raising the defaults
  (PENDING_N 10k to 100k, channel capacity 100 to 1000).
