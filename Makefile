.PHONY: all build test fmt release release-nix

all: fmt build test

build:
	cargo build

test:
	nix flake check

fmt:
	cargo fmt
	taplo fmt Cargo.toml taplo.toml deny.toml

release:
	git checkout main
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
	echo "Released $$new_version" && \
	echo "Run 'make release-nix' to update nix/default.nix"

release-nix:
	@version=$$(grep '^version = ".*"' Cargo.toml | head -1 | sed 's/.*"\([^"]*\)".*/\1/') && \
	echo "Updating nix/default.nix to version $$version" && \
	sed -i 's/version = ".*"/version = "'$$version'"/' nix/default.nix && \
	echo "Fetching source hash..." && \
	sed -i '/hash = "/{ /cargoHash/!s/hash = ".*"/hash = "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="/ }' nix/default.nix && \
	hash=$$(nix-build -E 'with import <nixpkgs> {}; callPackage ./nix/default.nix {}' 2>&1 | grep -oP 'got:\s+\Ksha256-[A-Za-z0-9+/=]+' | head -1) && \
	echo "Source hash: $$hash" && \
	sed -i '/hash = "/{ /cargoHash/!s/hash = ".*"/hash = "'$$hash'"/ }' nix/default.nix && \
	echo "Fetching cargo hash..." && \
	sed -i 's/cargoHash = ".*"/cargoHash = "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="/' nix/default.nix && \
	cargoHash=$$(nix-build -E 'with import <nixpkgs> {}; callPackage ./nix/default.nix {}' 2>&1 | grep -oP 'got:\s+\Ksha256-[A-Za-z0-9+/=]+' | head -1) && \
	echo "Cargo hash: $$cargoHash" && \
	sed -i 's/cargoHash = ".*"/cargoHash = "'$$cargoHash'"/' nix/default.nix && \
	echo "Verifying build..." && \
	nix-build -E 'with import <nixpkgs> {}; callPackage ./nix/default.nix {}' && \
	git add nix/default.nix && \
	git commit -m "build(nix): update default.nix for $$version" && \
	git push && \
	echo "nix/default.nix updated for $$version"
