BACKENDS ?= tree
BACKEND_FLAGS := $(foreach b,$(BACKENDS),--backend $(b))

.PHONY: build test unit blackbox clean

build:
	cargo build --release

# 一条命令运行全部测试：Rust 单元测试 + ZYX 黑盒测试
test: unit blackbox

unit:
	cargo test --quiet

blackbox: build
	python3 tests/run_tests.py $(BACKEND_FLAGS)

clean:
	cargo clean
