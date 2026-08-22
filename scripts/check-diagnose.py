#!/usr/bin/env python3
"""sqware diagnose.jsonl 校验：'#' 头跳过；每行 len 自洽（wire 不变量）；
严格 JSON 解析单独统计（事件行的 hex 字段如 "arg":0x... 是既有 wire 形状，
非严格 JSON——模块记录 halt/scene/watch 须通过严格解析）。"""
import json
import re
import sys

path = sys.argv[1] if len(sys.argv) > 1 else "sqware-diagnose.jsonl"
lines = json_ok = len_bad = 0
for ln, raw in enumerate(open(path, encoding="utf-8"), 1):
    line = raw.rstrip("\n")
    if not line or line.startswith("#"):
        continue
    lines += 1
    m = re.search(r'"len":(\d+)\}\s*$', line)
    if not m or int(m.group(1)) != len(line.encode()):
        len_bad += 1
        print(f"line {ln}: len field {m and m.group(1)} != bytes {len(line.encode())}")
        continue
    try:
        json.loads(line)
        json_ok += 1
    except json.JSONDecodeError:
        pass  # 既有事件 wire 的 hex 字段（详见文件头注释）
print(f"lines={lines} strict_json_ok={json_ok} not_strict={lines - json_ok} len_mismatch={len_bad}")
sys.exit(1 if len_bad else 0)