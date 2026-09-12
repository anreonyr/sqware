#!/bin/bash
# alloc-probe 的三条路（互斥；理由见 README 与 src/lib.rs 的接管段）。
#
#   ./run.sh smartalloc   # 默认：debug 档宿主堆由 smartalloc 接管（全局分配器）
#   ./run.sh plain        # 不接管：只看"用例过不过"（--no-default-features）
#   ./run.sh lsan         # LeakSanitizer 看宿主堆（必须不接管，否则块全被判 reachable）
#   ./run.sh orphan       # 演示：普通 Rust 分配漏掉 ⇒ 收尾转储点名
set -u
cd "$(dirname "$0")"
# 依赖落在工作区内的 CARGO_HOME：沙箱里 `~/.cargo/registry` 只读，拉 smartalloc 会失败
# （也顺带让这个 crate 自包含）。
export CARGO_HOME="${CARGO_HOME:-$(cd ../.. && pwd)/.cargo-home}"
mode=${1:-smartalloc}
# 把模式词从 `"$@"` 里摘掉：`cargo test` 后面的自由参数是**测试名过滤器**，
# 传 `smartalloc` 进去会把用例全过滤掉，跑出"0 passed / N filtered out"的假绿。
[ $# -gt 0 ] && shift
tgt=x86_64-unknown-linux-gnu
# 接管是默认 feature（`default = ["smartalloc"]`）；不接管一律走 `--no-default-features`。
case "$mode" in
  smartalloc)
    cargo test --target "$tgt" "$@"
    ;;
  plain)
    cargo test --target "$tgt" --no-default-features "$@"
    ;;
  lsan)
    # LSan 靠拦截 malloc/free：接管层把每个活块都挂在它自己的队列上 ⇒ LSan 一律判
    # "still reachable"（等于没测）。故这条路上关掉接管。
    RUSTFLAGS="-Zsanitizer=leak" cargo test --target "$tgt" --no-default-features "$@"
    ;;
  orphan)
    cargo run --target "$tgt" --example orphan "$@"
    ;;
  *)
    echo "用法: ./run.sh [smartalloc|plain|lsan|orphan]" >&2
    exit 2
    ;;
esac
