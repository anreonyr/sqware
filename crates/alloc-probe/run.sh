#!/bin/bash
# alloc-probe 的两个泄漏检测后端（互斥；理由见 README）。
#
#   ./run.sh lsan         # 默认：LeakSanitizer 看宿主堆（含分配器自己的元数据）
#   ./run.sh smartalloc   # 可选：smartalloc 当全局分配器（需能拉 crates.io）
#   ./run.sh plain        # 只跑用例（不起泄漏检测器，用来区分"用例失败"与"检测器报告"）
set -u
cd "$(dirname "$0")"
# 依赖落在工作区内的 CARGO_HOME：沙箱里 `~/.cargo/registry` 只读，拉 smartalloc 会失败
# （也顺带让这个 crate 自包含）。
export CARGO_HOME="${CARGO_HOME:-$(cd ../.. && pwd)/.cargo-home}"
mode=${1:-lsan}
# 把模式词从 `"$@"` 里摘掉：`cargo test` 后面的自由参数是**测试名过滤器**，
# 传 `lsan` 进去会把三条用例全过滤掉，跑出"0 passed / 3 filtered out"的假绿。
[ $# -gt 0 ] && shift
tgt=x86_64-unknown-linux-gnu
case "$mode" in
  lsan)
    RUSTFLAGS="-Zsanitizer=leak" cargo test --target "$tgt" "$@"
    ;;
  smartalloc)
    cargo test --target "$tgt" --features smartalloc
    ;;
  plain)
    cargo test --target "$tgt"
    ;;
  *)
    echo "用法: ./run.sh [lsan|smartalloc|plain]" >&2
    exit 2
    ;;
esac
