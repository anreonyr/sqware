#!/usr/bin/env python3
"""Round 3: comment normalization (precision edit, not mass delete)."""
import sys, re
from pathlib import Path

NAME = r'[A-Za-z_][\w]*(?:::[A-Za-z_][\w]*)*'
NAME_PHRASE = rf'(?:{NAME}(?:\s+{NAME})*)'

MODULE_HEADER_PATTERNS = [
    rf'^{NAME_PHRASE}\s*—\s*\*\*[^\xe3\x80\x95*]+\*\*\s*[(\xe3\x80\x28][^\xe3\x80\x29)]*[\xe3\x80\x29)]\s*[\xe3\x80\x9a:：]?\s*',
    rf'^{NAME_PHRASE}\s*—\s*\*\*[^\xe3\x80\x95*]+\*\*\s*[\xe3\x80\x9a:：]?\s*',
    rf'^{NAME_PHRASE}\s*—\s*[^*\xe3\x80\x95\n]+\s*[\xe3\x80\x9a:：]?\s*',
    rf'^{NAME_PHRASE}\s+(?:is|provides?|does|contains?|exposes?|holds?|owns?|manages?)\s+',
    rf'^{NAME_PHRASE}\s*[\xe3\x80\x9a:：]\s*',
    # `<module> 的那一半 —— rest` —— 中文所有格 + 长破折号（限长 rest ≤ 30 字 + 贪婪到下一个 `（`）
    rf'^{NAME_PHRASE}\s*有?的[^—\n]{{1,30}}?——\s*([^—\uFF08\uFF09\n]{{1,30}})',
    # `<module> 的<rest>` —— 中文所有格（无破折号）
    rf'^{NAME_PHRASE}\s*有?的[^—\n。]{2,30}\xe3\x80\x82?\s*',
]


def module_subject_rewrite(line: str) -> str:
    s = line.lstrip()
    if not (s.startswith('//!') or s.startswith('///')):
        return line
    prefix = '//!' if s.startswith('//!') else '///'
    body = s[len(prefix):].lstrip()
    for pat in MODULE_HEADER_PATTERNS:
        m = re.match(pat, body, re.IGNORECASE)
        if not m:
            continue
        groups = m.groups()
        # 有 capture group 时：用 capture 替换（rest 部分）—— 即使短
        if len(groups) > 0 and groups[0] is not None and len(groups[0].strip()) >= 1:
            body = groups[0].strip()
            break
        # 无 capture group 或 group 为空：用整体替换（去掉前缀）
        new_body = body[m.end():].lstrip()
        if len(new_body.strip()) > 5:
            body = new_body
            break
        # 整体替换后只剩不到 1 字符 — 跳过
    else:
        return line

    # 二次清理：开头 `（...）` 装饰
    if body.startswith('（'):
        idx = body.find('）', 1)
        if 0 < idx <= 30:
            after = body[idx + 1:].lstrip()
            if len(after) > 5:
                body = after
    body = body.rstrip()
    body = re.sub(r'[）)]+\s*$', '', body)
    body = re.sub(r'[）)]+[。.]+\s*$', '', body)
    body = re.sub(r'[。.]\s*$', '', body)
    body = re.sub(r'[，,;]+\s*$', '', body)
    body = body.strip()
    if not body:
        return ''
    return prefix + ' ' + body


def is_ascii_tree_block(lines, start):
    if start >= len(lines):
        return -1
    if '```' not in lines[start]:
        return -1
    j = start + 1
    has_tree = False
    has_deep = False
    closed = False
    max_scan = min(start + 30, len(lines))
    while j < max_scan:
        if '```' in lines[j]:
            closed = True
            break
        if any(c in lines[j] for c in ('├', '└', '│', '┌', '┐')):
            has_tree = True
        if re.search(r'^[/!]{2,}[ ]{2,}\S', lines[j]):
            has_deep = True
        j += 1
    if (has_tree or has_deep) and closed:
        return j + 1
    return -1


def is_table_remainder(line):
    s = line.lstrip()
    if not (s.startswith('///') or s.startswith('//')):
        return False
    body = s.lstrip('/').lstrip()
    return body.startswith('|')


def strip_nav_phrase(line):
    s = line.lstrip()
    if not s.startswith('///'):
        return line
    body = s[3:].lstrip()
    if not body:
        return ''
    body = re.sub(r'（同[^）]*crate[ ]?::[^）]*）', '', body)
    body = re.sub(r'\(同[^)]*crate[ ]?::[^)]*\)', '', body)
    body = re.sub(r'（crate[ ]?::[a-z_:]+ 那一格）', '', body)
    body = re.sub(r'\(crate[ ]?::[a-z_:]+ 那一格\)', '', body)
    body = re.sub(r'（crate[ ]?::[a-z_:]+）', '', body)
    body = re.sub(r'\(crate[ ]?::[a-z_:]+\)', '', body)
    body = re.sub(r'见[ ]+super[ ]?::[ ]?[a-z_:]+', '', body)
    body = re.sub(r'见[ ]+crate[ ]?::[ ]?[a-z_:]+', '', body)
    body = re.sub(r'同[ ]+super[ ]?::[ ]?[a-z_:]+', '', body)
    body = re.sub(r'同[ ]+crate[ ]?::[ ]?[a-z_:]+', '', body)
    body = re.sub(r'\s+', ' ', body).strip()
    body = re.sub(r'[，,。.;;:；：：∶]+\s*$', '', body)
    if not body:
        return ''
    return '/// ' + body


def is_orphan_comment(line):
    s = line.lstrip()
    # 不带 /// 前缀的孤立残句（cross-module link 删剩的）：行首是 `:` 后跟字母
    if re.match(r'^:\w+', s):
        return True
    if not (s.startswith('///') or s.startswith('//!')):
        return False
    body = s.lstrip('/').lstrip()
    if body.startswith('——而') or body.startswith('——（') or body.startswith('——(') or body.startswith('——见') or body.startswith('——同'):
        return True
    if body.startswith('——'):
        return True
    if body.startswith('（') and '）' not in body:
        return True
    if body.startswith(':') and re.match(r'^:\w+', body):
        return True
    if body.startswith('**') and body.endswith('）') and '（' not in body:
        return True
    if re.match(r'^一个落点\*\*（[^）]*）\.?\s*$', body):
        return True
    if '——' in body and body.endswith('时') and not body.endswith('。'):
        return True
    return False


def is_orphan_block(lines, start):
    """检测 /// 或 //! 注释块是否含孤儿行 —— 若是，整段删。"""
    if start >= len(lines):
        return -1
    if not (lines[start].lstrip().startswith('//') or lines[start].lstrip().startswith('///') or lines[start].lstrip().startswith('//!')):
        return -1
    prefix = '//!' if lines[start].lstrip().startswith('//!') else '///'
    # 找出连续 /// 或 //! 行
    j = start
    has_orphan = False
    block_count = 0
    while j < len(lines):
        s = lines[j].lstrip()
        if s.startswith('///') or s.startswith('//!'):
            if is_orphan_comment(lines[j]):
                has_orphan = True
            block_count += 1
            j += 1
        elif re.match(r'^:\w+', s):
            # `:xxx` 残句行 —— 视作块的一部分
            has_orphan = True
            block_count += 1
            j += 1
        else:
            break
    if has_orphan and block_count > 0:
        return start + block_count
    return -1


def strip_inline_nav(line):
    s = line.lstrip()
    if not s.startswith('///'):
        return line
    body = s[3:].lstrip()
    if not body:
        return ''
    body = re.sub(r'见[ ]+[A-Z][\w]*(::[A-Z][\w]*)+', '', body)
    body = re.sub(r'见[ ]+\w+::\w+', '', body)
    body = re.sub(r'同[ ]+[A-Z][\w]*(::[A-Z][\w]*)+', '', body)
    body = re.sub(r'同[ ]+\w+::\w+', '', body)
    body = re.sub(r'见[ ]+它?自己?的?注', '', body)
    body = re.sub(r'见[ ]+super[ ]?::[ ]?[a-z_:]+', '', body)
    body = re.sub(r'见[ ]+crate[ ]?::[ ]?[a-z_:]+', '', body)
    body = re.sub(r'\s+', ' ', body).strip()
    if not body:
        return ''
    return '/// ' + body


def strip_broken_continuation(line):
    """删除以 — 而/——而 / 行内残破续接 开头/中间 的半句话。"""
    s = line.lstrip()
    if not (s.startswith('///') or s.startswith('//!')):
        return line
    body = s.lstrip('/').lstrip()
    # 整行内容只是 `——而 ... 时` 单一破句
    if body.startswith('——而') or body.startswith('——（'):
        if len(body) < 80:
            return ''
    # 整行以 "出来" / "一个落点" 等孤立破词结尾
    return line


def process_file(path):
    text = path.read_text(encoding='utf-8')
    lines = text.split('\n')
    out = []
    removed = 0
    rewritten = 0
    i = 0
    while i < len(lines):
        line = lines[i]

        if (line.lstrip().startswith('//') or line.lstrip().startswith('///') or line.lstrip().startswith('//!')):
            end = is_ascii_tree_block(lines, i)
            if end > 0:
                has_tree_char = False
                has_deep = False
                for k in range(i + 1, end - 1):
                    if any(c in lines[k] for c in ('├', '└', '│', '┌', '┐')):
                        has_tree_char = True
                    if re.search(r'^[/!]{2,}[ ]{2,}\S', lines[k]):
                        has_deep = True
                if has_tree_char or has_deep:
                    removed += (end - i)
                    i = end
                    continue

        if is_table_remainder(line):
            removed += 1
            i += 1
            continue

        if line.lstrip().startswith('//!') or line.lstrip().startswith('///'):
            new_line = module_subject_rewrite(line)
            if new_line == '':
                removed += 1
                i += 1
                continue
            if new_line != line:
                rewritten += 1
                # 检查 new_line 是否变 orphan
                if is_orphan_comment(new_line):
                    # new_line 是 orphan — 用 new_line 替换 line，标记为待删除
                    # 简单做法：替换为 new_line，但立刻进入 orphan 流程
                    # 用临时变量传新 line 到 orphan 分支
                    line = new_line
                    # 不 append 到 out，留给 orphan 分支处理
                    # 注意：i 暂不动，但需要 continue 到 orphan 分支
                else:
                    out.append(new_line)
                    i += 1
                    continue

        if is_orphan_comment(line):
            # 检查是否在多行 /// 块内 — 是则整段删
            end = is_orphan_block(lines, i)
            if end > 0:
                removed += (end - i)
                i = end
                continue
            # 向前看：如果是 /// 块尾部，下一块含 :xxx 残句，整段删
            # 找到本 /// 块的起点
            block_start = i
            while block_start > 0:
                s_prev = lines[block_start - 1].lstrip()
                if s_prev.startswith('///') or s_prev.startswith('//!'):
                    block_start -= 1
                else:
                    break
            if block_start < i:
                end2 = is_orphan_block(lines, block_start)
                if end2 > 0:
                    removed += (end2 - block_start)
                    i = end2
                    continue
            removed += 1
            i += 1
            continue

        if line.lstrip().startswith('///'):
            new_line = strip_nav_phrase(line)
            if new_line == '':
                removed += 1
                i += 1
                continue
            if new_line != line:
                rewritten += 1
                out.append(new_line)
                i += 1
                continue

        if line.lstrip().startswith('///'):
            new_line = strip_inline_nav(line)
            if new_line == '':
                removed += 1
                i += 1
                continue
            if new_line != line:
                rewritten += 1
                out.append(new_line)
                i += 1
                continue

        if line.lstrip().startswith('///') or line.lstrip().startswith('//!'):
            new_line = strip_broken_continuation(line)
            if new_line == '':
                removed += 1
                i += 1
                continue
            if new_line != line:
                rewritten += 1
                out.append(new_line)
                i += 1
                continue

        out.append(line)
        i += 1

    new_text = '\n'.join(out)
    new_text = re.sub(r'\n{3,}', '\n\n', new_text)
    if new_text != text:
        path.write_text(new_text, encoding='utf-8')
    return removed, rewritten


def main():
    args = sys.argv[1:] if len(sys.argv) > 1 else ['.']
    files = []
    for a in args:
        p = Path(a)
        if p.is_file():
            files.append(p)
        elif p.is_dir():
            files.extend(sorted(p.rglob('*.rs')))
    files = sorted(set(files))
    total_removed = 0
    total_rewritten = 0
    files_touched = 0
    for path in files:
        r, w = process_file(path)
        if r > 0 or w > 0:
            files_touched += 1
            try:
                rel = path.relative_to(Path.cwd())
            except ValueError:
                rel = path
            print(f"  - {rel}: 删 {r} 行 / 改 {w} 行")
        total_removed += r
        total_rewritten += w
    print(f"\n总删 {total_removed} 行 / 改 {total_rewritten} 行（{files_touched} 文件）")


if __name__ == '__main__':
    main()