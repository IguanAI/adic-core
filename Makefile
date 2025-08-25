# ADIC Core Makefile

.PHONY: help build test run clean docker-build docker-run docker-stop simulation

help:
	@echo "ADIC Core Development Commands"
	@echo "=============================="
	@echo "  make build          - Build the project in release mode"
	@echo "  make test           - Run all tests"
	@echo "  make run            - Run a local node"
	@echo "  make clean          - Clean build artifacts"
	@echo "  make docker-build   - Build Docker image"
	@echo "  make docker-run     - Run single node in Docker"
	@echo "  make docker-multi   - Run multi-node setup"
	@echo "  make docker-stop    - Stop all Docker containers"
	@echo "  make simulation     - Run Python simulator"
	@echo "  make docs           - Generate documentation"

# Rust commands
build:
	cargo build --release

test:
	cargo test --all

run:
	./target/release/adic start

clean:
	cargo clean
	rm -rf data/

# Docker commands
docker-build:
	docker-compose build

docker-run:
	docker-compose up -d adic-node

docker-multi:
	docker-compose --profile multi-node up -d

docker-monitoring:
	docker-compose --profile monitoring up -d

docker-stop:
	docker-compose down

docker-logs:
	docker-compose logs -f

# Simulation
simulation:
	docker-compose --profile simulation up simulator

simulation-notebook:
	cd simulation && jupyter notebook

# Development
dev:
	cargo watch -x check -x test -x run

fmt:
	cargo fmt --all

clippy:
	cargo clippy --all -- -D warnings

docs:
	cargo doc --open --no-deps

# Installation
install-deps:
	rustup update
	rustup component add rustfmt clippy
	cargo install cargo-watch

install-python-deps:
	pip install -r simulation/requirements.txt

# Testing
unit-test:
	cargo test --lib --all

integration-test:
	cargo test --test '*'

bench:
	cargo bench

# Local development
local-init:
	mkdir -p data
	./target/release/adic init -o data

local-keygen:
	./target/release/adic keygen -o data/node.key

local-test:
	./target/release/adic test --count 100

# CI/CD helpers
ci-test:
	cargo fmt -- --check
	cargo clippy --all -- -D warnings
	cargo test --all
	cargo build --release

# Performance testing
perf-test:
	cd simulation && python performance_analysis.py

# Clean everything
clean-all: clean
	docker-compose down -v
	rm -rf data/ simulation/__pycache__ simulation/*.json