SHELL := /usr/bin/env bash
.SHELLFLAGS := -eu -o pipefail -c

EXTENSION_DIR := $(abspath .)
ROOT_DIR := $(abspath ../..)
GRAMMAR_DIR := $(EXTENSION_DIR)/grammar

TARGET_DIR := $(EXTENSION_DIR)/target
WASM_TARGET := wasm32-wasip2
WASM_PROFILE := release

CRATE_NAME := kome_zed
WASM_SOURCE := $(TARGET_DIR)/$(WASM_TARGET)/$(WASM_PROFILE)/$(CRATE_NAME).wasm
WASM_OUTPUT := $(EXTENSION_DIR)/extension.wasm

DIST_DIR := $(EXTENSION_DIR)/dist
STAGE_DIR := $(DIST_DIR)/kome-zed

VERSION := $(shell sed -n 's/^version = "\(.*\)"/\1/p' extension.toml | head -n 1)
ARCHIVE := $(DIST_DIR)/kome-zed-$(VERSION).zip

.PHONY: all
all: build

.PHONY: help
help:
	@echo "Kome Zed extension"
	@echo
	@echo "Targets:"
	@echo "  make prepare   Install required Rust target and npm packages"
	@echo "  make grammar   Generate the Tree-sitter parser"
	@echo "  make lsp       Build kome-lsp"
	@echo "  make extension Build extension.wasm"
	@echo "  make build     Build grammar, LSP and extension"
	@echo "  make dev       Build everything for Zed dev installation"
	@echo "  make package   Create a ZIP archive under dist/"
	@echo "  make check     Run validation checks"
	@echo "  make clean     Remove generated build artifacts"

.PHONY: check-tools
check-tools:
	@command -v cargo >/dev/null || { echo "cargo is required"; exit 1; }
	@command -v rustup >/dev/null || { echo "rustup is required"; exit 1; }
	@command -v npm >/dev/null || { echo "npm is required"; exit 1; }
	@command -v zip >/dev/null || { echo "zip is required"; exit 1; }

.PHONY: prepare
prepare: check-tools
	rustup target add $(WASM_TARGET)
	cd "$(GRAMMAR_DIR)" && \
		if [[ -f package-lock.json ]]; then \
			npm ci; \
		else \
			npm install; \
		fi

.PHONY: grammar
grammar:
	cd "$(GRAMMAR_DIR)" && npm run generate
	@test -f "$(GRAMMAR_DIR)/src/parser.c" || { \
		echo "Tree-sitter parser was not generated"; \
		exit 1; \
	}
	@echo "Generated Tree-sitter parser"

.PHONY: lsp
lsp:
	cargo build \
		--manifest-path "$(ROOT_DIR)/Cargo.toml" \
		-p kome_lsp

	@test -x "$(ROOT_DIR)/target/debug/kome-lsp" || { \
		echo "kome-lsp was not generated"; \
		exit 1; \
	}
	@echo "Built $(ROOT_DIR)/target/debug/kome-lsp"

.PHONY: extension
extension:
	rustup target add $(WASM_TARGET)

	RUSTC_WRAPPER="" cargo build \
		--manifest-path "$(EXTENSION_DIR)/Cargo.toml" \
		--target "$(WASM_TARGET)" \
		--release \
		--target-dir "$(TARGET_DIR)"

	@test -f "$(WASM_SOURCE)" || { \
		echo "WASM output not found: $(WASM_SOURCE)"; \
		exit 1; \
	}

	cp "$(WASM_SOURCE)" "$(WASM_OUTPUT)"
	@echo "Built $(WASM_OUTPUT)"

.PHONY: build
build: grammar lsp extension
	@echo
	@echo "Kome Zed extension built successfully"
	@echo "Extension directory: $(EXTENSION_DIR)"
	@echo "Language server:    $(ROOT_DIR)/target/debug/kome-lsp"
	@echo "WASM extension:     $(WASM_OUTPUT)"

.PHONY: dev
dev: prepare build
	@echo
	@echo "Open Zed and run:"
	@echo "  zed: install dev extension"
	@echo
	@echo "Then select:"
	@echo "  $(EXTENSION_DIR)"

.PHONY: check
check: grammar
	cargo check \
		--manifest-path "$(EXTENSION_DIR)/Cargo.toml"

	cargo check \
		--manifest-path "$(ROOT_DIR)/Cargo.toml" \
		-p kome_lsp

	cd "$(GRAMMAR_DIR)" && \
		printf 'component App {}\n' > .kome-check.kome && \
		npx tree-sitter parse .kome-check.kome && \
		rm .kome-check.kome

	@echo "All checks passed"

.PHONY: package
package: build
	rm -rf "$(STAGE_DIR)"
	mkdir -p "$(STAGE_DIR)"

	cp "$(EXTENSION_DIR)/extension.toml" "$(STAGE_DIR)/"
	cp "$(WASM_OUTPUT)" "$(STAGE_DIR)/"
	cp -R "$(EXTENSION_DIR)/languages" "$(STAGE_DIR)/"
	cp -R "$(EXTENSION_DIR)/grammar" "$(STAGE_DIR)/"

	rm -rf "$(STAGE_DIR)/grammar/node_modules"
	rm -rf "$(STAGE_DIR)/grammar/build"

	cd "$(DIST_DIR)" && \
		rm -f "$(ARCHIVE)" && \
		zip -r "$(ARCHIVE)" "$(notdir $(STAGE_DIR))"

	@echo "Created $(ARCHIVE)"
	@echo
	@echo "Note: this ZIP is an archive for distribution or CI."
	@echo "Zed dev installation uses the extension directory directly."

.PHONY: clean
clean:
	rm -rf "$(TARGET_DIR)"
	rm -rf "$(DIST_DIR)"
	rm -f "$(WASM_OUTPUT)"
	rm -rf "$(GRAMMAR_DIR)/build"
	rm -rf "$(GRAMMAR_DIR)/node_modules"