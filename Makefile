.PHONY: all build test fmt release

all: fmt build test

build:
	cargo build

test:
	nix flake check

fmt:
	cargo fmt
	taplo fmt Cargo.toml taplo.toml deny.toml

release:
	@git diff --quiet || (echo "Error: working tree has uncommitted changes" && exit 1)
	@current=$$(grep '^version = ".*"' Cargo.toml | head -1 | sed 's/.*"\([^"]*\)".*/\1/') && \
	major=$$(echo $$current | cut -d. -f1) && \
	minor=$$(echo $$current | cut -d. -f2) && \
	patch=$$(echo $$current | cut -d. -f3) && \
	new_patch=$$((patch + 1)) && \
	new_version="$$major.$$minor.$$new_patch" && \
	sed -i "s/^version = \"$$current\"/version = \"$$new_version\"/" Cargo.toml && \
	echo "Bumped version: $$current -> $$new_version" && \
	cargo generate-lockfile --quiet && \
	make fmt && \
	git add Cargo.toml Cargo.lock && \
	git commit -m "chore: bump version to $$new_version" && \
	git tag "$$new_version" && \
	git push && git push origin "$$new_version" && \
	echo "Released $$new_version"
