.PHONY: check fmt test clippy

check:
	cargo check --workspace

fmt:
	cargo fmt --all

test:
	cargo test --workspace

clippy:
	cargo clippy --workspace --all-targets -- -D warnings
