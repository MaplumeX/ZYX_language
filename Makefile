BACKENDS ?= tree vm
BACKEND_FLAGS := $(foreach b,$(BACKENDS),--backend $(b))

.PHONY: build test unit blackbox bench clean

build:
	cargo build --release

# 一条命令运行全部测试：Rust 单元测试 + ZYX 黑盒测试
test: unit blackbox

unit:
	cargo test --quiet

blackbox: build
	python3 tests/run_tests.py $(BACKEND_FLAGS)

# 性能测试：ZYX 两个后端与 Python 对比，结果写入 bench/results.md 并重新生成图表
bench: build
	python3 bench/run_bench.py
	python3 bench/plot.py

clean:
	cargo clean
