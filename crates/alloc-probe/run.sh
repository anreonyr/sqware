#!/bin/bash
# alloc-probe 的路（互斥；理由见 README 与 src/lib.rs 的接管段）。
#
# 单线程（宿主单元测试）
#   ./run.sh smartalloc   # 默认：孤儿缓冲转储（总量面）
#   ./run.sh mockalloc    # 第一层·甲：配对判词（逐笔账）
#   ./run.sh dhat         # 第一层·乙：宿主堆用量（气压计）
# 多线程（扩展）
#   ./run.sh mt           # 并发层：守恒 / 模式写读 / per-hart 池 / 跨 hart 回路 + dropcount
#   ./run.sh tsan         # 数据竞争：ThreadSanitizer
#   ./run.sh miri         # UB + 数据竞争：Miri（规模自动缩小，见 tests/*.rs 的 cfg(miri)）
#   ./run.sh bench        # 并发压力/吞吐量：malloc-bench-rs（Larson / mstress）× 内核适配器
# 零后端 / 其它
#   ./run.sh plain        # 不接管：只看用例过不过（--no-default-features）
#   ./run.sh lsan         # LeakSanitizer（必须不接管；覆盖单线程 + 多线程两层的泄漏）
#   ./run.sh orphan       # 演示：普通 Rust 分配漏掉 ⇒ 收尾转储点名
set -u
cd "$(dirname "$0")"
# 依赖落在工作区内的 CARGO_HOME：沙箱里 `~/.cargo/registry` 只读，拉依赖会失败
# （也顺带让这个 crate 自包含）。
export CARGO_HOME="${CARGO_HOME:-$(cd ../.. && pwd)/.cargo-home}"
mode=${1:-smartalloc}
# 把模式词从 `"$@"` 里摘掉：`cargo test` 后面的自由参数是**测试名过滤器**，
# 传 `smartalloc` 进去会把用例全过滤掉，跑出"0 passed / N filtered out"的假绿。
[ $# -gt 0 ] && shift
tgt=x86_64-unknown-linux-gnu
# 前三条是**三个互斥的宿主堆后端**（`#[global_allocator]` 只能有一个，lib.rs 里有
# `compile_error!` 挡着"同时开两个"）；其余是零后端路。
#
# `--no-default-features` 的用法：默认 feature 是 `smartalloc`，只要不是它就得关掉，
# 否则两个接管点同时在场（编译期就报错，见 lib.rs）。并发那层更硬：smartalloc 的 C 层
# 是无锁全局链表，多线程会写坏它 ⇒ `tests/mt.rs` 在 `smartalloc` 档下整个不编进来。
case "$mode" in
  smartalloc)
    cargo test --target "$tgt" "$@"
    ;;
  mockalloc)
    # 第一层·甲：`tests/pairing.rs`（proptest 的幕，每幕读一次配对判词）。
    cargo test --target "$tgt" --no-default-features --features mockalloc "$@"
    ;;
  dhat)
    # 第一层·乙：`tests/heap.rs`（dhat 的 testing 档，一条用例三段判据）。
    cargo test --target "$tgt" --no-default-features --features dhat "$@"
    ;;
  mt)
    # 多线程（扩展）：只跑并发那一层。
    cargo test --target "$tgt" --no-default-features --test mt "$@"
    ;;
  tsan)
    # 数据竞争：ThreadSanitizer。**必须 `-Zbuild-std`**（连 std 一起插桩重编）：
    # Rust 的 `std::sync::Mutex` 走 futex 而不是 pthread，它给 TSan 的同步注解写在 std 源码里
    # （`#[cfg(sanitize = "thread")]`）—— 预编译 std 没插桩，那些注解就被编掉，TSan 于是
    # **看不见我们的锁**，把"同一把锁下的两次访问"全报成竞争（实测：42 条，全是假阳性，
    # 两份栈都在 `FrameAllocator::allocate` 里、争用地址是 `frame.rs:165` 的 freelist 数组）。
    # 重编 std 之后竞争报告才可信（只剩真竞争）。
    RUSTFLAGS="-Zsanitizer=thread" \
      cargo test -Zbuild-std --target "$tgt" --no-default-features --test mt "$@"
    ;;
  miri)
    # UB + 数据竞争：Miri（解释执行）。`-Zmiri-permissive-provenance` 是必需的：分配器
    # 把帧地址当 `usize` 来回搬（`frame_addr`/`ptr.addr()`），严格 provenance 下那类
    # int→ptr 转换会被判非法 —— 内核在真机上本来就靠物理地址恒等映射，这条放宽是**如实**
    # 而不是掩盖。规模：`tests/*.rs` 里 `cfg(miri)` 把语料与线程数压到秒级。
    MIRIFLAGS="-Zmiri-permissive-provenance" \
      cargo miri test --target "$tgt" --no-default-features "$@"
    ;;
  bench)
    # 并发压力/吞吐量：release 档 + 不接管（起多线程，接管层的 C 列表是单线程的）。
    cargo run --release --target "$tgt" --no-default-features --example throughput "$@"
    ;;
  plain)
    cargo test --target "$tgt" --no-default-features "$@"
    ;;
  lsan)
    # LSan 靠拦截 malloc/free：接管层把每个活块都挂在它自己的队列上 ⇒ LSan 一律判
    # "still reachable"（等于没测）。故这条路上关掉接管。LSan 是线程感知的，
    # 故它同时覆盖两层：单线程用例 + `tests/mt.rs` 的并发用例。
    RUSTFLAGS="-Zsanitizer=leak" cargo test --target "$tgt" --no-default-features "$@"
    ;;
  orphan)
    cargo run --target "$tgt" --example orphan "$@"
    ;;
  *)
    echo "用法: ./run.sh [smartalloc|mockalloc|dhat|mt|tsan|miri|bench|plain|lsan|orphan]" >&2
    exit 2
    ;;
esac
