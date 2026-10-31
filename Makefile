BACKENDS ?= tree vm
BACKEND_FLAGS := $(foreach b,$(BACKENDS),--backend $(b))

.PHONY: build test unit blackbox apptest bench clean

build:
	cargo build --release

# 一条命令运行全部测试：Rust 单元测试 + ZYX 黑盒测试 + 应用程序自测
test: unit blackbox apptest

unit:
	cargo test --quiet

blackbox: build
	python3 tests/run_tests.py $(BACKEND_FLAGS)

# 2048 应用的自测模式（首行输入 test），两个后端都要通过
apptest: build
	@for b in $(BACKENDS); do \
		out=$$(printf 'test\n' | ./target/release/zyx --backend $$b examples/app/2048.zyx); \
		if [ "$$out" = "self-test passed" ]; then echo "[$$b] examples/app/2048.zyx: $$out"; \
		else echo "[$$b] examples/app/2048.zyx 自测失败: $$out"; exit 1; fi; \
	done

# 性能测试：ZYX 两个后端与 Python 对比，结果写入 bench/results.md 并重新生成图表
bench: build
	python3 bench/run_bench.py
	python3 bench/plot.py

clean:
	cargo clean
