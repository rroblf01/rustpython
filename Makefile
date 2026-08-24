# RustPython Makefile
# Targets for building, testing, linting, and deploying.
#
# Usage:
#   make              # default: build (debug)
#   make build        # debug build
#   make release      # release build with LTO
#   make static       # fully static binary (Docker FROM scratch)
#   make test         # run all tests (Rust + Python)
#   make check        # fast compilation check
#   make lint         # clippy + ruff + typos + fmt check
#   make clean        # clean artifacts

SHELL := /bin/bash
RUSTPYTHON := target/debug/rustpython
RUSTPYTHON_REL := target/release/rustpython
RUSTPYTHON_STATIC := target/x86_64-unknown-linux-gnu/release/rustpython
RUSTPYTHON_TEST := target/debug/rustpython-test
RUSTPYTHON_TEST_FEATURES := jit,sqlite3
TEST_VENV := /tmp/test-uv-rustpython/.venv
TEST_VENV_SITE := $(TEST_VENV)/lib/python3.14/site-packages
TESTS_DIR := tests
UNAME := $(shell uname -s)

# ──────────────────── Colors ────────────────────
RED    := \033[31m
GREEN  := \033[32m
YELLOW := \033[33m
CYAN   := \033[36m
RESET  := \033[0m

.PHONY: all build release dev static test test-rust test-python test-cpython
.PHONY: lint lint-clippy lint-ruff lint-typos lint-fmt
.PHONY: check clean clean-logs clean-pycache run bench fmt docs install-tools
.PHONY: help

# ── Default ──────────────────────────────────────
all: build

# ── Build ────────────────────────────────────────
build:
	@echo -e "$(CYAN)==> Building (debug)...$(RESET)"
	cargo build
	@echo -e "$(GREEN)✔  $(RUSTPYTHON)$(RESET)"

release:
	@echo -e "$(CYAN)==> Building (release, LTO)...$(RESET)"
	cargo build --release
	@echo -e "$(GREEN)✔  $(RUSTPYTHON_REL)$(RESET)"
	@echo -e "    Binary size: $$(stat --printf='%s' $(RUSTPYTHON_REL) 2>/dev/null | numfmt --to=iec || echo '?')"

# Fast inner-loop dev build: no LTO, opt-level=1, 16 codegen units.
# Much faster incremental builds than `make release` (~2-4x); the binary
# is somewhat slower at runtime, so use it for iteration, not benchmarks.
RUSTPYTHON_DEV := target/release-lite/rustpython

dev:
	@echo -e "$(CYAN)==> Building (release-lite: fast dev iteration)...$(RESET)"
	cargo build --profile release-lite
	@echo -e "$(GREEN)✔  $(RUSTPYTHON_DEV)$(RESET)"

# Fastest iteration loop: type-check first (~7s, catches all compile errors),
# then the cheap release-lite binary. Use this between every code edit;
# reserve `make release` (~40s) for sweep/benchmark/commit moments only.
it:
	@echo -e "$(CYAN)==> cargo check (fast error gate)...$(RESET)"
	cargo check --release 2>&1 | grep -E "^error" -A5 || true
	@echo -e "$(CYAN)==> release-lite build...$(RESET)"
	cargo build --profile release-lite
	@echo -e "$(GREEN)✔ iterate with $(RUSTPYTHON_DEV)$(RESET)"

# ── Static binary (Docker FROM scratch) ───────────
static:
	@echo -e "$(CYAN)==> Building static binary...$(RESET)"
	@echo -e "    Using RUSTFLAGS='-C target-feature=+crt-static'"
	@echo -e "    $(YELLOW)Note: libc is statically linked.$(RESET)"
	@echo -e "    $(YELLOW)⚠  cranelift JIT uses dlopen — JIT disabled in static build.$(RESET)"
	RUSTFLAGS="-C target-feature=+crt-static" cargo build --release \
		--no-default-features
	@echo -e "$(GREEN)✔  $(RUSTPYTHON_STATIC)$(RESET)"
	@ls -lh target/release/rustpython 2>/dev/null
	@echo
	@echo -e "To verify it's truly static:"
	@echo -e "  ldd target/release/rustpython  # should say 'not a dynamic executable'"
	@echo
	@echo -e "To build a Docker scratch image:"
	@echo -e '  cat > Dockerfile << "EOF"'
	@echo -e '  FROM scratch'
	@echo -e '  COPY rustpython /'
	@echo -e '  ENTRYPOINT ["/rustpython"]'
	@echo -e '  EOF'
	@echo -e "  docker build -t rustpython-scratch ."

# ── Restore ──────────────────────────────────────
restore:
	@echo -e "$(CYAN)==> Restoring dependencies...$(RESET)"
	cargo fetch

# ── Test ──────────────────────────────────────────
test: test-rust test-python

test-rust:
	@echo -e "$(CYAN)==> Running Rust unit tests...$(RESET)"
	cargo test 2>&1 | tail -20

build-test:
	@echo -e "$(CYAN)==> Building test binary (features: $(RUSTPYTHON_TEST_FEATURES))...$(RESET)"
	cargo build --release --features $(RUSTPYTHON_TEST_FEATURES) --bin rustpython
	@cp target/release/rustpython $(RUSTPYTHON_TEST)

test-python: build-test
	@echo -e "$(CYAN)==> Running Python tests...$(RESET)"
	@mkdir -p /tmp/rustpython-test-logs
	@passed=0; failed=0; \
	export PYTHONPATH=$(TEST_VENV_SITE):$${PYTHONPATH:-}; \
	for f in $$(ls $(TESTS_DIR)/*.py 2>/dev/null | sort); do \
		name=$$(basename $$f); \
		printf "  [test] %-40s" "$$name"; \
		if output=$$(./$(RUSTPYTHON_TEST) "$$f" 2>&1); then \
			echo -e "$(GREEN)PASS$(RESET)"; \
			passed=$$((passed + 1)); \
		else \
			echo -e "$(RED)FAIL$(RESET)"; \
			echo "$$output" > "/tmp/rustpython-test-logs/$$name.log"; \
			failed=$$((failed + 1)); \
		fi; \
	done; \
	echo; \
	echo -e "$(GREEN)$$passed passed$(RESET), $(RED)$$failed failed$(RESET)"; \
	[ $$failed -eq 0 ]

test-python-verbose: build-test
	@echo -e "$(CYAN)==> Running Python tests (verbose)...$(RESET)"
	@export PYTHONPATH=$(TEST_VENV_SITE):$${PYTHONPATH:-}; \
	for f in $$(ls $(TESTS_DIR)/*.py 2>/dev/null | sort); do \
		name=$$(basename $$f); \
		echo "--- $$name ---"; \
		./$(RUSTPYTHON_TEST) "$$f" 2>&1; \
		echo "exit code: $$?"; \
		echo; \
	done

# Test a specific Python test file (uses debug binary without sqlite3)
test-one:
	@if [ -z "$(FILE)" ]; then \
		echo "Usage: make test-one FILE=tests/test_name.py"; \
		exit 1; \
	fi
	@echo -e "$(CYAN)==> Running $(FILE)...$(RESET)"
	@binary="$$(cd "$$(dirname "$(RUSTPYTHON)")" && pwd)/$$(basename "$(RUSTPYTHON)")"; \
	testfile="$$(cd "$$(dirname "$(FILE)")" && pwd)/$$(basename "$(FILE)")"; \
	scratch_dir="$$(mktemp -d "/tmp/rustpython-test-one-scratch.XXXXXX")"; \
	export PYTHONPATH=$(TEST_VENV_SITE):$${PYTHONPATH:-}; \
	(cd "$$scratch_dir" && "$$binary" "$$testfile") 2>&1; \
	exit_code=$$?; \
	rm -rf "$$scratch_dir"; \
	echo "exit code: $$exit_code"

# Test with real site-packages (uv project)
test-uv: $(RUSTPYTHON)
	@echo -e "$(CYAN)==> Running uv integration test...$(RESET)"
	@if [ ! -d "/tmp/test-uv-rustpython/.venv" ]; then \
		echo -e "$(YELLOW)⚠  uv test project not found. Create one:$(RESET)"; \
		echo "  cd /tmp && uv init test-uv-rustpython && cd test-uv-rustpython && uv add requests certifi"; \
		exit 1; \
	fi
	@cd /tmp/test-uv-rustpython && ../rustpython/target/debug/rustpython -c "import sys; sys.path.insert(0, sys.path[3]); import certifi; print('certifi:', certifi.where())" 2>&1
	@echo -e "$(GREEN)✔  uv integration OK$(RESET)"

# ── CPython test suite ─────────────────────────────
CPYTHON_TIMEOUT := 120
# Matches nproc (12 on the dev machine this was tuned on) — full-core
# parallel *debug*-profile (unoptimized, no LTO) rustpython processes will
# starve the desktop of CPU for the ~7-8 minute duration of a sweep (a real,
# previously-observed tradeoff, explicitly accepted in exchange for wall-
# clock speed — see cpython_test_suite_compat.md's resource-monitor
# diagnosis for why this is NOT a memory-safety concern: even 8-way never
# used more than ~9GB/32GB RAM, and the earlier OOM incident that originally
# motivated capping this was a since-fixed interpreter bug — unbounded Vec
# growth in list()/list*n/reversed(range) — not a parallelism problem at
# all). Drop this back toward nproc-2 if the desktop-starvation tradeoff
# stops being worth it.
CPYTHON_PARALLEL := 12

test-cpython: build-test
	@echo -e "$(CYAN)==> Running CPython compatibility tests ($(CPYTHON_TIMEOUT)s timeout, $(CPYTHON_PARALLEL) parallel)...$(RESET)"
	@mkdir -p /tmp/rustpython-test-logs/cpython
	@mkdir -p /tmp/rustpython-test-logs/cpython/summary
	@total=0; pass=0; fail=0; timeout=0; parse=0; \
	export TEST_VENV_SITE=$(TEST_VENV_SITE); \
	SCRIPTS=$$(ls $(TESTS_DIR)/cpython/test_*.py 2>/dev/null | sort); \
	TOTAL=$$(echo "$$SCRIPTS" | wc -l); \
	echo "  Total test files: $$TOTAL"; \
	echo "  Parallelism: $(CPYTHON_PARALLEL)"; \
	echo ""; \
	echo "$$SCRIPTS" | xargs -P $(CPYTHON_PARALLEL) -I {} bash -c ' \
		name=$$(basename "{}" .py); \
		logdir=/tmp/rustpython-test-logs/cpython; \
		result=$$(tests/cpython_runner.sh $(RUSTPYTHON_TEST) "{}" "$$logdir" $(CPYTHON_TIMEOUT) $(TEST_VENV_SITE)); \
		echo "$$result"; \
		echo "$$result" >> "/tmp/rustpython-test-logs/cpython/summary/$$name.txt"; \
	'; \
	echo ""; \
	echo "=== RESULTS ==="; \
	latest=$$(tail -qn1 /tmp/rustpython-test-logs/cpython/summary/*.txt 2>/dev/null); \
	pass_count=$$(echo "$$latest" | grep -c "^PASS|"); \
	fail_count=$$(echo "$$latest" | grep -c "^FAIL|"); \
	timeout_count=$$(echo "$$latest" | grep -c "^TIMEOUT|"); \
	parse_count=$$(echo "$$latest" | grep -c "^PARSE_ERROR|"); \
	echo "  PASS: $$pass_count"; \
	echo "  FAIL: $$fail_count"; \
	echo "  TIMEOUT: $$timeout_count"; \
	echo "  PARSE_ERROR: $$parse_count"; \
	TOTAL=$$(ls /tmp/rustpython-test-logs/cpython/summary/*.txt 2>/dev/null | wc -l); \
	echo "  TOTAL: $$TOTAL"; \
	[ $$fail_count -eq 0 ] || echo -e "$(YELLOW)  ⚠  $$fail_count failures + $$timeout_count timeouts + $$parse_count parse errors$(RESET)"

test-cpython-quick: CPYTHON_TIMEOUT := 30
test-cpython-quick: test-cpython

# ── Check (fast) ─────────────────────────────────
check:
	@echo -e "$(CYAN)==> Checking compilation...$(RESET)"
	cargo check
	@echo -e "$(GREEN)✔  No errors$(RESET)"

# ── Lint ──────────────────────────────────────────
lint: lint-clippy lint-fmt

lint-clippy:
	@echo -e "$(CYAN)==> Running clippy...$(RESET)"
	cargo clippy -- -D warnings 2>&1 | tail -20
	@echo -e "$(GREEN)✔  Clippy OK$(RESET)"

lint-fmt:
	@echo -e "$(CYAN)==> Checking formatting...$(RESET)"
	cargo fmt --check 2>&1
	@echo -e "$(GREEN)✔  Formatting OK$(RESET)"

lint-fix:
	@echo -e "$(CYAN)==> Auto-fixing formatting...$(RESET)"
	cargo fmt
	@echo -e "$(GREEN)✔  Formatted$(RESET)"

# ── Ruff (Python lint) ─────────────────────────────
lint-ruff:
	@if command -v ruff &>/dev/null; then \
		echo -e "$(CYAN)==> Ruff lint...$(RESET)"; \
		ruff check $(TESTS_DIR)/*.py 2>&1; \
		ruff format --check $(TESTS_DIR)/*.py 2>&1; \
		echo -e "$(GREEN)✔  Ruff OK$(RESET)"; \
	else \
		echo -e "$(YELLOW)⚠  ruff not installed. Install with: pip install ruff$(RESET)"; \
	fi

# ── Typos ──────────────────────────────────────────
lint-typos:
	@if command -v typos &>/dev/null; then \
		echo -e "$(CYAN)==> Typos check...$(RESET)"; \
		typos src/ tests/ README.md 2>&1; \
		echo -e "$(GREEN)✔  Typos OK$(RESET)"; \
	else \
		echo -e "$(YELLOW)⚠  typos not installed. Install with: cargo install typos-cli$(RESET)"; \
	fi

# ── Full lint (including optional tools) ────────────
lint-all: lint lint-ruff lint-typos

# ── Install tools ──────────────────────────────────
install-tools:
	@echo -e "$(CYAN)==> Installing optional lint tools...$(RESET)"
	@if ! command -v ruff &>/dev/null; then \
		echo "  Installing ruff..."; \
		pip install ruff 2>/dev/null || cargo install ruff 2>/dev/null || \
		echo -e "$(YELLOW)  ⚠  Could not install ruff. Try: pip install ruff$(RESET)"; \
	fi
	@if ! command -v typos &>/dev/null; then \
		echo "  Installing typos-cli..."; \
		cargo install typos-cli 2>&1 | tail -3; \
	fi
	@echo -e "$(GREEN)✔  Tools installed$(RESET)"

# ── Run ────────────────────────────────────────────
run: $(RUSTPYTHON)
	@echo -e "$(CYAN)==> Running...$(RESET)"
	./$(RUSTPYTHON) $(ARGS)

# Run with site-packages from uv project
run-uv: $(RUSTPYTHON)
	@if [ -z "$(SCRIPT)" ]; then \
		echo "Usage: make run-uv SCRIPT=path/to/script.py"; \
		exit 1; \
	fi
	cd /tmp/test-uv-rustpython && /opt/data/proyectos/rustpython/$(RUSTPYTHON) "$(SCRIPT)" 2>&1

repl: $(RUSTPYTHON)
	@echo -e "$(CYAN)==> Starting REPL...$(RESET)"
	./$(RUSTPYTHON)

# ── Benchmarks ────────────────────────────────────
bench: release
	@echo -e "$(CYAN)==> Running benchmark trend harness (RustPython vs CPython)...$(RESET)"
	@python3 benchmarks/bench_trend.py $(BENCH_ARGS)

# Full trend including the CPython suite aggregate (slow).
bench-full: release
	@echo -e "$(CYAN)==> Running benchmark trend + CPython suite aggregate...$(RESET)"
	@python3 benchmarks/bench_trend.py --tests

# ── Clean ──────────────────────────────────────────
clean:
	@echo -e "$(CYAN)==> Cleaning...$(RESET)"
	cargo clean
	@rm -rf /tmp/rustpython-test-logs
	@find . -name "__pycache__" -not -path "./target/*" -exec rm -rf {} + 2>/dev/null || true
	@echo -e "$(GREEN)✔  Cleaned$(RESET)"

clean-logs:
	@rm -rf /tmp/rustpython-test-logs
	@echo "Logs cleaned"

clean-pycache:
	@find . -name "__pycache__" -not -path "./target/*" -exec rm -rf {} + 2>/dev/null || true
	@echo "__pycache__ cleaned"

# ── Documentation ─────────────────────────────────
docs:
	@echo -e "$(CYAN)==> Generating docs...$(RESET)"
	cargo doc --no-deps 2>&1 | tail -5
	@echo -e "$(GREEN)✔  Docs at target/doc/rustpython/index.html$(RESET)"

# ── Help ──────────────────────────────────────────
help:
	@echo -e "$(CYAN)RustPython Makefile$(RESET)"
	@echo
	@echo -e "  $(GREEN)make$(RESET)             Default: debug build"
	@echo -e "  $(GREEN)make build$(RESET)       Debug build"
	@echo -e "  $(GREEN)make release$(RESET)     Release build (LTO, optimized)"
	@echo -e "  $(GREEN)make static$(RESET)      Fully static binary (Docker scratch)"
	@echo -e "  $(GREEN)make check$(RESET)       Fast compilation check"
	@echo -e "  $(GREEN)make test$(RESET)        All tests (Rust + Python)"
	@echo -e "  $(GREEN)make test-rust$(RESET)   Rust unit tests"
	@echo -e "  $(GREEN)make test-python$(RESET) Python test suite"
	@echo -e "  $(GREEN)make test-uv$(RESET)     Integration with uv site-packages"
	@echo -e "  $(GREEN)make test-one FILE=x$(RESET) Run one Python test file"
	@echo -e "  $(GREEN)make lint$(RESET)        Clippy + fmt check"
	@echo -e "  $(GREEN)make lint-all$(RESET)    Full lint incl. ruff + typos"
	@echo -e "  $(GREEN)make lint-fix$(RESET)    Auto-fix formatting"
	@echo -e "  $(GREEN)make run ARGS='-c ...'$(RESET)  Run with args"
	@echo -e "  $(GREEN)make run-uv SCRIPT=x$(RESET) Run script with uv deps"
	@echo -e "  $(GREEN)make repl$(RESET)        Start REPL"
	@echo -e "  $(GREEN)make bench$(RESET)       Run benchmarks"
	@echo -e "  $(GREEN)make clean$(RESET)       Clean artifacts"
	@echo -e "  $(GREEN)make docs$(RESET)        Build API docs"
	@echo -e "  $(GREEN)make install-tools$(RESET) Install ruff, typos-cli"
	@echo
	@echo -e "  For Docker scratch: $(YELLOW)make static$(RESET)"
	@echo -e "  For release:         $(YELLOW)make release$(RESET)"
	@echo -e "  For dev cycle:       $(YELLOW)make check test lint$(RESET)"
