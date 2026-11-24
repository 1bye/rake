#!/usr/bin/env bash
set -eu

check_tree() {
    local tree
    tree=$(cargo tree -p rake-core -e normal --locked --prefix none "$@")

    if grep -E '^[^[:space:]]*gpui' <<<"$tree"; then
        echo "rake-core has a GPUI dependency (${*:-native})" >&2
        return 1
    fi
}

check_tree
check_tree --target wasm32-unknown-unknown

# The web-crash example uses the facade's default features and dev-dependencies.
# Skip host build dependencies and proc macros, which can use native crates.
for package in rake-core rake-app; do
    tree=$(cargo tree -p "$package" -e normal,dev,no-proc-macro --locked \
        --prefix none --target wasm32-unknown-unknown)
    if grep -E '^errno v' <<<"$tree"; then
        echo "$package has an errno dependency (wasm32-unknown-unknown)" >&2
        exit 1
    fi
done
