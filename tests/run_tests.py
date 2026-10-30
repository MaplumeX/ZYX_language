#!/usr/bin/env python3
"""ZYX 黑盒测试运行器。

递归收集 tests/zyx/ 下的 .zyx 文件，用解释器逐个运行，并按文件中的注释指令比对结果：

    # expect: <文本>        标准输出的下一行应为 <文本>（按出现顺序逐行比对）
    # expect-error: <文本>  标准错误应为 <文本>，且退出码非 0
    # expect-exit: <数字>   退出码应为 <数字>
    # stdin: <文本>         作为标准输入的一行喂给程序

没有 expect-error 的测试要求退出码为 0 且标准错误为空。比对时忽略行尾空白。

用法：python3 tests/run_tests.py [--bin PATH] [--backend tree] [--backend vm] [过滤词]
"""

import argparse
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
TEST_DIR = ROOT / "tests" / "zyx"
DIRECTIVE = re.compile(r"#\s*(expect|expect-error|expect-exit|stdin):\s?(.*)$")


def parse_directives(text):
    spec = {"stdout": [], "stderr": None, "exit": None, "stdin": []}
    for line in text.splitlines():
        m = DIRECTIVE.search(line)
        if not m:
            continue
        kind, value = m.group(1), m.group(2).rstrip()
        if kind == "expect":
            spec["stdout"].append(value)
        elif kind == "expect-error":
            spec["stderr"] = value
        elif kind == "expect-exit":
            spec["exit"] = int(value)
        else:
            spec["stdin"].append(value)
    return spec


def run_one(binary, backend, path):
    """返回失败原因列表，空列表表示通过。"""
    spec = parse_directives(path.read_text(encoding="utf-8"))
    stdin = "".join(line + "\n" for line in spec["stdin"])
    try:
        proc = subprocess.run(
            [str(binary), "--backend", backend, str(path)],
            input=stdin.encode(),
            capture_output=True,
            timeout=10,
        )
    except subprocess.TimeoutExpired:
        return ["超时（10 秒）"]

    stdout = proc.stdout.decode("utf-8", "replace").replace("\r\n", "\n")
    stderr = proc.stderr.decode("utf-8", "replace").strip()
    # 行尾空白不参与比较（编辑器常会删掉注释行尾的空格）
    actual_lines = [line.rstrip() for line in stdout.split("\n")]
    if actual_lines and actual_lines[-1] == "":
        actual_lines.pop()

    problems = []
    if actual_lines != spec["stdout"]:
        problems.append(diff_lines(spec["stdout"], actual_lines))

    if spec["stderr"] is not None:
        if stderr != spec["stderr"]:
            problems.append(f"标准错误不符\n    期望: {spec['stderr']}\n    实际: {stderr or '(空)'}")
        if proc.returncode == 0:
            problems.append("期望出错，但退出码为 0")
    elif stderr:
        problems.append(f"意外的标准错误: {stderr}")

    expected_exit = spec["exit"]
    if expected_exit is None and spec["stderr"] is None:
        expected_exit = 0
    if expected_exit is not None and proc.returncode != expected_exit:
        problems.append(f"退出码不符：期望 {expected_exit}，实际 {proc.returncode}")
    return problems


def diff_lines(expected, actual):
    out = ["标准输出不符"]
    for i in range(max(len(expected), len(actual))):
        e = expected[i] if i < len(expected) else "(无)"
        a = actual[i] if i < len(actual) else "(无)"
        mark = "  " if e == a else "✗ "
        out.append(f"    {mark}第 {i + 1} 行  期望: {e}")
        if e != a:
            out.append(f"    {' ' * len(mark)}{' ' * (len(str(i + 1)) + 4)}实际: {a}")
    return "\n".join(out)


def main():
    ap = argparse.ArgumentParser(description="运行 ZYX 黑盒测试")
    ap.add_argument("--bin", default=str(ROOT / "target" / "release" / "zyx"), help="解释器路径")
    ap.add_argument("--backend", action="append", choices=["tree", "vm"], help="要测试的后端，可重复")
    ap.add_argument("filter", nargs="?", default="", help="只运行路径中包含该词的测试")
    args = ap.parse_args()

    binary = pathlib.Path(args.bin)
    if not binary.exists():
        sys.exit(f"找不到解释器 {binary}，请先运行 cargo build --release")
    backends = args.backend or ["tree"]
    tests = sorted(p for p in TEST_DIR.rglob("*.zyx") if args.filter in str(p.relative_to(TEST_DIR)))
    if not tests:
        sys.exit("没有找到测试")

    total = passed = 0
    for backend in backends:
        for path in tests:
            total += 1
            name = f"[{backend}] {path.relative_to(TEST_DIR)}"
            problems = run_one(binary, backend, path)
            if problems:
                print(f"FAIL {name}")
                for p in problems:
                    print("  " + p.replace("\n", "\n  "))
            else:
                passed += 1

    failed = total - passed
    print(f"\n通过 {passed} / 共 {total}" + (f"，失败 {failed}" if failed else ""))
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
