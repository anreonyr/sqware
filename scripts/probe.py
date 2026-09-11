#!/usr/bin/env python3
"""把 §10.32 的临时探针**按行**插进/删除（正文改动不走整段替换——那是本会话切坏文件三次的原因）。"""
import sys
from pathlib import Path

CLIENT = Path("crates/protocol/src/console/client.rs")

# 探针：readline 推请求前 / 收到回复后 各打一次 guest 时钟
PROBE = '''        print_ms("before-pull");
'''
PUSH_PROBE = '''        print_ms("after-push");
'''

def insert():
    s = CLIENT.read_text(encoding="utf-8")
    anchor = "        let mut buf = [0u8; MSG_LEN];\n        // 阻塞读（不是 `pull_timeout`）：等多久由用户决定。"
    assert anchor in s, "锚点缺失"
    s = s.replace(anchor, PUSH_PROBE + anchor)
    anchor2 = "        HolePie::from_token(self.reply_mine).pull(&mut buf)?;"
    assert anchor2 in s
    s = s.replace(anchor2, anchor2 + "\n" + PROBE.rstrip("\n"))
    # 辅助：打印 guest 时钟（秒.纳秒 → 毫秒整数）
    helper = '''
/// 临时探针（§10.32）：把 guest 时钟打到设备上，格式 `<tag>:<ms>`。
fn print_ms(tag: &str) {
    if let Ok((secs, nanos)) = runtime::env::chrono::clock() {
        let ms: u64 = secs * 1000 + nanos / 1_000_000;
        let mut b = [0u8; 20];
        let mut v = ms;
        let mut i = b.len();
        loop {
            i -= 1;
            b[i] = b'0' + (v % 10) as u8;
            v /= 10;
            if v == 0 {
                break;
            }
        }
        let _ = runtime::env::io::put(tag);
        if let Ok(t) = core::str::from_utf8(&b[i..]) {
            let _ = runtime::env::io::put(t);
        }
        let _ = runtime::env::io::put(" ");
    }
}
'''
    s = s.replace("fn denied() -> erra::Error<EnvError> {", helper + "\nfn denied() -> erra::Error<EnvError> {")
    CLIENT.write_text(s, encoding="utf-8")
    print("探针已插")

def remove():
    s = CLIENT.read_text(encoding="utf-8")
    s = s.replace(PUSH_PROBE, "").replace(PROBE, "")
    i = s.find("/// 临时探针（§10.32）：把 guest 时钟打到设备上")
    j = s.find("fn denied() -> erra::Error<EnvError> {", i)
    if i > 0 and j > i:
        s = s[:i] + s[j:]
    CLIENT.write_text(s, encoding="utf-8")
    print("探针已删")

if __name__ == "__main__":
    {"insert": insert, "remove": remove}[sys.argv[1]]()
