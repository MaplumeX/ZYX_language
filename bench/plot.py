#!/usr/bin/env python3
"""根据 bench/results.json 生成性能对比图 docs/images/benchmark.svg（无第三方依赖）。

图中每个用例两根横条：树遍历版、虚拟机版相对 Python 的耗时倍数；虚线处为 Python（= 1）。
"""

import json
import pathlib

ROOT = pathlib.Path(__file__).resolve().parent.parent
OUT = ROOT / "docs" / "images" / "benchmark.svg"

# 参考调色板：同一色相的浅、深两级表示"优化前 / 优化后"
SERIES = [("tree", "树遍历版", "#86b6ef", "#5598e7"), ("vm", "字节码虚拟机", "#2a78d6", "#3987e5")]

W = 760
LEFT = 120          # 用例名称列
PLOT_X0, PLOT_X1 = LEFT + 16, W - 72
BAR_H, BAR_GAP, GROUP_GAP = 14, 2, 18
TOP = 104


def bar_path(x0, y, w, h, r=4):
    """左端贴住基线、右端（数据端）圆角的横条。"""
    r = min(r, w, h / 2)
    return (f"M{x0:.1f},{y:.1f}H{x0 + w - r:.1f}A{r},{r} 0 0 1 {x0 + w:.1f},{y + r:.1f}"
            f"V{y + h - r:.1f}A{r},{r} 0 0 1 {x0 + w - r:.1f},{y + h:.1f}H{x0:.1f}Z")


def main():
    data = json.loads((ROOT / "bench" / "results.json").read_text())
    rows = sorted(data["results"], key=lambda r: r["vm"] / r["python"], reverse=True)
    ratios = [(r["case"], r["tree"] / r["python"], r["vm"] / r["python"]) for r in rows]
    vmax = max(t for _, t, _ in ratios)
    axis_max = int(vmax) + 1
    scale = (PLOT_X1 - PLOT_X0) / axis_max
    x = lambda v: PLOT_X0 + v * scale

    group_h = 2 * BAR_H + BAR_GAP
    plot_h = len(ratios) * group_h + (len(ratios) - 1) * GROUP_GAP
    H = TOP + plot_h + 56

    vm_lo, vm_hi = min(v for *_, v in ratios), max(v for *_, v in ratios)
    tr_lo, tr_hi = min(t for _, t, _ in ratios), max(t for _, t, _ in ratios)
    title = f"字节码虚拟机把与 Python 的差距缩小到 {vm_lo:.1f}–{vm_hi:.1f} 倍"
    subtitle = f"ZYX 耗时 ÷ Python 耗时，越短越好。树遍历版为 {tr_lo:.1f}–{tr_hi:.1f} 倍；虚线处与 Python 持平"

    out = [f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" width="{W}" height="{H}" role="img" aria-label="{title}">',
           """<style>
  .surface { fill: #fcfcfb; }
  .ink { fill: #0b0b0b; }
  .muted { fill: #52514e; }
  .grid { stroke: #e4e3df; }
  .axis { stroke: #8b8a85; }
  .ref { stroke: #52514e; }
  .s-tree { fill: #86b6ef; }
  .s-vm { fill: #2a78d6; }
  .halo { stroke: #fcfcfb; stroke-width: 3px; paint-order: stroke; stroke-linejoin: round; }
  text { font-family: -apple-system, "Segoe UI", "PingFang SC", "Noto Sans CJK SC", "Microsoft YaHei", sans-serif; }
  @media (prefers-color-scheme: dark) {
    .surface { fill: #1a1a19; }
    .ink { fill: #f0efec; }
    .muted { fill: #c3c2b7; }
    .grid { stroke: #383835; }
    .axis { stroke: #77766f; }
    .ref { stroke: #c3c2b7; }
    .s-tree { fill: #5598e7; }
    .s-vm { fill: #3987e5; }
    .halo { stroke: #1a1a19; }
  }
</style>""",
           f'<rect class="surface" width="{W}" height="{H}"/>',
           f'<text class="ink" x="24" y="32" font-size="17" font-weight="600">{title}</text>',
           f'<text class="muted" x="24" y="54" font-size="12.5">{subtitle}</text>']

    # 图例
    lx = 24
    for key, label, _, _ in SERIES:
        out.append(f'<rect class="s-{key}" x="{lx}" y="70" width="12" height="12" rx="2"/>')
        out.append(f'<text class="ink" x="{lx + 18}" y="80.5" font-size="12.5">{label}</text>')
        lx += 18 + len(label) * 13 + 24

    # 网格与刻度
    y_axis = TOP + plot_h + 10
    for t in range(axis_max + 1):
        gx = x(t)
        out.append(f'<line class="grid" x1="{gx:.1f}" x2="{gx:.1f}" y1="{TOP - 6}" y2="{y_axis}" stroke-width="1"/>')
        out.append(f'<text class="muted" x="{gx:.1f}" y="{y_axis + 18}" font-size="11.5" text-anchor="middle">{t}×</text>')
    out.append(f'<line class="axis" x1="{PLOT_X0}" x2="{PLOT_X0}" y1="{TOP - 6}" y2="{y_axis}" stroke-width="1"/>')

    # Python 参考线（画在条形下层）
    rx = x(1)
    out.append(f'<line class="ref" x1="{rx:.1f}" x2="{rx:.1f}" y1="{TOP - 10}" y2="{y_axis}" stroke-width="1.25" stroke-dasharray="4 3"/>')
    out.append(f'<text class="muted" x="{rx + 4:.1f}" y="{TOP - 14}" font-size="11.5">Python</text>')

    # 数据
    for i, (case, tree, vm) in enumerate(ratios):
        gy = TOP + i * (group_h + GROUP_GAP)
        out.append(f'<text class="ink" x="{LEFT}" y="{gy + group_h / 2 + 4.5:.1f}" font-size="13" text-anchor="end">{case}</text>')
        for j, (key, label, _, _) in enumerate(SERIES):
            v = tree if key == "tree" else vm
            by = gy + j * (BAR_H + BAR_GAP)
            out.append(f'<path class="s-{key}" d="{bar_path(PLOT_X0, by, v * scale, BAR_H)}"><title>{case} · {label}：{v:.2f}×（Python = 1）</title></path>')
            weight = ' font-weight="600"' if key == "vm" else ""
            out.append(f'<text class="ink halo" x="{x(v) + 6:.1f}" y="{by + BAR_H - 3}" font-size="11.5"{weight}>{v:.2f}×</text>')


    env = data["environment"]
    out.append(f'<text class="muted" x="24" y="{H - 12}" font-size="11">数据：bench/results.json · {env["cpu"]} · Python {env["python"]} · 每项 {data["runs"]} 次取中位数 · {env["date"]}</text>')
    out.append("</svg>")

    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text("\n".join(out) + "\n")
    print(f"已生成 {OUT}")


if __name__ == "__main__":
    main()
