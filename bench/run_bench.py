#!/usr/bin/env python3
"""ZYX 性能测试：对每个用例分别运行 ZYX（各后端）和 Python，取多次运行的中位数。

每个用例由 bench/zyx/<名字>.zyx 和 bench/py/<名字>.py 组成，两者输出必须完全一致，
否则该用例记为失败。结果写入 bench/results.md 和 bench/results.json。

用法：python3 bench/run_bench.py [--runs 5] [--backend tree] [--backend vm] [过滤词]
"""

import argparse
import datetime
import json
import pathlib
import platform
import statistics
import subprocess
import sys
import time

ROOT = pathlib.Path(__file__).resolve().parent.parent
BENCH = ROOT / "bench"
ZYX_BIN = ROOT / "target" / "release" / "zyx"


def timed(cmd):
    start = time.perf_counter()
    proc = subprocess.run(cmd, capture_output=True, timeout=600)
    elapsed = time.perf_counter() - start
    if proc.returncode != 0:
        raise RuntimeError(f"{' '.join(map(str, cmd))} 退出码 {proc.returncode}: {proc.stderr.decode(errors='replace').strip()}")
    return elapsed, proc.stdout.decode(errors="replace")


def measure(cmd, runs):
    times, output = [], None
    for _ in range(runs):
        t, out = timed(cmd)
        times.append(t)
        output = out
    return statistics.median(times), min(times), output


def environment():
    def run(cmd):
        try:
            return subprocess.run(cmd, capture_output=True, text=True).stdout.strip()
        except OSError:
            return "unknown"

    cpu = "unknown"
    try:
        for line in pathlib.Path("/proc/cpuinfo").read_text().splitlines():
            if line.startswith("model name"):
                cpu = line.split(":", 1)[1].strip()
                break
    except OSError:
        pass
    return {
        "date": datetime.date.today().isoformat(),
        "cpu": cpu,
        "os": platform.platform(),
        "python": platform.python_version(),
        "rustc": run(["rustc", "--version"]),
    }


def main():
    ap = argparse.ArgumentParser(description="运行 ZYX 性能测试")
    ap.add_argument("--runs", type=int, default=5, help="每个用例运行次数，取中位数")
    ap.add_argument("--backend", action="append", choices=["tree", "vm"], help="要测试的后端，可重复")
    ap.add_argument("filter", nargs="?", default="", help="只运行名字包含该词的用例")
    args = ap.parse_args()

    if not ZYX_BIN.exists():
        sys.exit(f"找不到 {ZYX_BIN}，请先运行 cargo build --release")
    backends = args.backend or ["tree", "vm"]
    cases = sorted(p.stem for p in (BENCH / "zyx").glob("*.zyx") if args.filter in p.stem)

    results = []
    for name in cases:
        row = {"case": name}
        py_time, py_min, py_out = measure([sys.executable, BENCH / "py" / f"{name}.py"], args.runs)
        row["python"] = py_time
        ok = True
        for b in backends:
            t, _, out = measure([ZYX_BIN, "--backend", b, BENCH / "zyx" / f"{name}.zyx"], args.runs)
            row[b] = t
            if out != py_out:
                ok = False
                print(f"警告：{name} 的 {b} 后端输出与 Python 不一致", file=sys.stderr)
        row["output_matches"] = ok
        results.append(row)
        cells = "  ".join(f"{b} {row[b]:.3f}s ({row[b] / py_time:.2f}x)" for b in backends)
        print(f"{name:<12} python {py_time:.3f}s  {cells}")

    env = environment()
    write_markdown(results, backends, env, args.runs)
    (BENCH / "results.json").write_text(
        json.dumps({"environment": env, "runs": args.runs, "results": results}, indent=2, ensure_ascii=False) + "\n"
    )
    print(f"\n结果已写入 {BENCH / 'results.md'}")


def write_markdown(results, backends, env, runs):
    lines = [
        "# 性能测试原始数据",
        "",
        f"- 日期：{env['date']}",
        f"- CPU：{env['cpu']}",
        f"- 系统：{env['os']}",
        f"- Python：{env['python']}",
        f"- Rust：{env['rustc']}",
        f"- 每个用例运行 {runs} 次，取墙钟时间中位数（含进程启动）",
        "",
        "倍数 = ZYX 耗时 / Python 耗时，小于 1 表示比 Python 快。",
        "",
        "| 用例 | Python (s) | " + " | ".join(f"ZYX {b} (s) | {b} 倍数" for b in backends) + " | 输出一致 |",
        "|---|---|" + "---|---|" * len(backends) + "---|",
    ]
    for r in results:
        cells = " | ".join(f"{r[b]:.3f} | {r[b] / r['python']:.2f}" for b in backends)
        lines.append(f"| {r['case']} | {r['python']:.3f} | {cells} | {'是' if r['output_matches'] else '否'} |")
    (BENCH / "results.md").write_text("\n".join(lines) + "\n")


if __name__ == "__main__":
    main()
