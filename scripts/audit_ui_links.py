#!/usr/bin/env python3
"""前端按钮链路体检：DOM id / 事件监听 / 内联 onclick / mock 覆盖 / 命令契约。

用法：python3 scripts/audit_ui_links.py [--strict]
退出码 1 表示发现需修问题（--strict 时对“疑似”也失败）。
"""
import os, re, sys, json

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SRC = os.path.join(ROOT, 'src')
MAIN = os.path.join(ROOT, 'src-tauri', 'src', 'main.rs')

def read(p):
    with open(p, encoding='utf-8') as f:
        return f.read()

# ---------------- 1. 命令契约 ----------------
def parse_commands():
    src = read(MAIN); lines = src.split('\n')
    cmds = {}
    for i, l in enumerate(lines):
        if not l.strip().startswith('#[tauri::command'):
            continue
        attr = l; j = i
        while not attr.rstrip().endswith(']') and j < len(lines) - 1:
            j += 1; attr += ' ' + lines[j].strip()
        rename = 'camel'
        m = re.search(r'rename_all\s*=\s*"(\w+)"', attr)
        if m: rename = m.group(1)
        k = j + 1
        while k < len(lines) and k < j + 8 and not re.search(r'\bfn\s+\w+', lines[k]):
            k += 1
        sig = ' '.join(lines[k:k + 10])
        m = re.search(r'\bfn\s+(\w+)\s*\(', sig)
        if not m: continue
        name = m.group(1); pos = sig.index('(', m.end() - 1)
        depth = 0; params = []; cur = ''; stopped = False
        for ch in sig[pos:]:
            if ch == '(':
                depth += 1
                if depth > 1: cur += ch
                continue
            if ch == '<':
                depth += 1; cur += ch; continue
            if ch in ')>':
                depth -= 1
                if depth == 0:
                    params.append(cur); stopped = True; break
                cur += ch; continue
            if depth == 1 and ch == ',':
                params.append(cur); cur = ''; continue
            cur += ch
        plist = []
        for p in params:
            p = p.strip()
            if not p or ':' not in p: continue
            pname, ptype = p.split(':', 1)
            pname = pname.strip().removeprefix('mut ').strip()
            ptype = ptype.strip()
            if re.search(r'\b(State|AppHandle|Window|WebviewWindow|Webview|Managed)\b\s*(<|$)', ptype):
                continue
            plist.append((pname, ptype.startswith('Option<')))
        cmds[name] = {'rename': rename, 'params': plist}
    reg = set()
    for m in re.finditer(r'generate_handler!\s*\[([^\]]*?)\]', src, re.S):
        for n in re.split(r'[,\s]+', m.group(1)):
            n = n.strip()
            if n and n in cmds: reg.add(n)
    return cmds, reg

def camel(k):
    parts = k.split('_')
    return parts[0] + ''.join(p.capitalize() for p in parts[1:])

def frontend_calls():
    out = []
    for f in sorted(os.listdir(SRC)):
        if not f.endswith('.js'): continue
        path = os.path.join(SRC, f); text = read(path)
        for m in re.finditer(r"invoke\s*\(\s*['\"]([A-Za-z0-9_]+)['\"]", text):
            name = m.group(1); start = text.index('(', m.start())
            depth = 0; i = start; end = None
            while i < len(text):
                if text[i] == '(': depth += 1
                elif text[i] == ')':
                    depth -= 1
                    if depth == 0: end = i; break
                i += 1
            inner = text[start + 1:end]
            line = text[:m.start()].count('\n') + 1
            out.append({'file': f, 'line': line, 'cmd': name, 'args': inner})
    return out

def audit_contract(cmds, reg, calls):
    issues = []; notes = []
    for c in calls:
        n = c['cmd']
        # 注释里的示例调用不算
        if c.get('in_comment'): continue
        if n not in cmds:
            issues.append(f"{c['file']}:{c['line']} invoke('{n}') 后端无此命令"); continue
        if n not in reg:
            issues.append(f"{c['file']}:{c['line']} invoke('{n}') 命令未注册进 generate_handler!")
        d = cmds[n]
        want = [camel(p) if d['rename'] == 'camel' else p for p, opt in d['params'] if not opt]
        # 取出第二个参数对象的顶层键
        args = c['args']
        comma = None; depth = 0; q = None
        for k, ch in enumerate(args):
            if q:
                if ch == q and args[k - 1] != '\\': q = None
                continue
            if ch in '"\'': q = ch; continue
            if ch in '([{': depth += 1
            elif ch in ')]}': depth -= 1
            elif ch == ',' and depth == 0: comma = k; break
        second = args[comma + 1:].strip() if comma is not None else ''
        if not second:
            if want: issues.append(f"{c['file']}:{c['line']} invoke('{n}') 未传参，后端必填 {want}")
            continue
        if not second.startswith('{'):
            continue  # 变量 / 表达式传参：不在静态审计范围
        keys = []
        d2 = 0; q = None; cur = ''
        for ch in second:
            if q:
                if ch == q and cur[-1:] != '\\': q = None
                cur += ch; continue
            if ch in '"\'': q = ch; cur += ch; continue
            if ch == '{':
                d2 += 1
                if d2 > 1: cur += ch
                continue
            if ch == '}':
                d2 -= 1
                if d2 == 0: keys.append(cur); break
                cur += ch; continue
            if d2 == 1 and ch == ',':
                keys.append(cur); cur = ''; continue
            if d2 >= 1: cur += ch
        top = []
        for piece in keys:
            piece = piece.strip()
            if not piece: continue
            mm = re.match(r'([A-Za-z_$][\w$]*)\s*(:|$)', piece)
            top.append(mm.group(1) if mm else ('...' if piece.startswith('...') else '?expr'))
        if the('...') in top or any(t.startswith('?') for t in top):
            notes.append(f"{c['file']}:{c['line']} invoke('{n}') 参数含展开/表达式，跳过静态比对")
            continue
        missing = [k for k in want if k not in top]
        if missing:
            issues.append(f"{c['file']}:{c['line']} invoke('{n}') 缺必填 {missing}")
    return issues, notes

def the(x): return x

# ---------------- 2. DOM id ----------------
def html_ids(page):
    text = read(os.path.join(SRC, page))
    ids = set(re.findall(r'\bid="([^"]+)"', text))
    fn_names = set(re.findall(r'\b(?:function|const|let|var)\s+([A-Za-z_$][\w$]*)', read(os.path.join(SRC, page.replace('.html', '.js')))) if os.path.exists(os.path.join(SRC, page.replace('.html','.js'))) else set())
    return text, ids, fn_names

def audit_dom():
    issues = []; notes = []
    pairs = [('console.html', 'console.js'), ('pet.html', 'pet.js'), ('panel.html', 'panel.js'),
             ('guard.html', 'guard.js'), ('menu.html', None)]
    for html, js in pairs:
        text = read(os.path.join(SRC, html))
        ids = set(re.findall(r'\bid="([^"]+)"', text))
        if js:
            code = read(os.path.join(SRC, js))
            # JS 里动态注入的模板也可能带 id，一并算作该页可用 id
            ids |= set(re.findall(r"""id=["']([^"']+)["']""", code))
            ids |= set(re.findall(r"""\.id\s*=\s*["']([^"']+)["']""", code))
            for m in re.finditer(r"\$\('([^']+)'\)|document\.getElementById\('([^']+)'\)", code):
                idv = m.group(1) or m.group(2)
                line = code[:m.start()].count('\n') + 1
                if idv not in ids:
                    issues.append(f"{js}:{line} 引用的 #{idv} 在 {html} 里不存在")
            # 内联 onclick 引用的函数是否在本页 js 里定义
            for m in re.finditer(r'on(?:click|change|input)="([A-Za-z_$][\w$]*)\(', text):
                fn = m.group(1)
                if re.search(rf'\b(?:function\s+{re.escape(fn)}|(?:const|let|var)\s+{re.escape(fn)}\s*=|{re.escape(fn)}\s*[:=]\s*(?:async\s*)?(?:function|\()', code):
                    continue
                if re.search(rf'\b{re.escape(fn)}\s*=', code) or re.search(rf'\b{re.escape(fn)}\b', code):
                    notes.append(f"{html} 内联 onclick={fn}() 未在 {js} 找到显式定义（可能挂在 window 上）")
                else:
                    issues.append(f"{html} 内联 onclick={fn}() 在 {js} 中未定义")
    # 按钮有无绑定
    for html, js in [('console.html', 'console.js'), ('pet.html', 'pet.js'), ('panel.html', 'panel.js'), ('guard.html', 'guard.js')]:
        text = read(os.path.join(SRC, html)); code = read(os.path.join(SRC, js))
        for m in re.finditer(r'<button[^>]*id="([^"]+)"[^>]*>', text):
            bid = m.group(1)
            line = text[:m.start()].count('\n') + 1
            if f"'{bid}'" not in code and f'"{bid}"' not in code:
                issues.append(f"{html}:{line} 按钮 #{bid} 在 {js} 中没有任何引用（可能点了没反应）")
    return issues, notes

# ---------------- 3. 事件名 ----------------
# 只由浏览器 harness / mock 广播的事件：真机后端从不发，前端挂着监听是为演示链路。
# （smoke.html 的「一键冒烟」按钮会 broadcast 这些事件，用来在浏览器里跑通 UI 分支。）
MOCK_ONLY_NOTE = "仅 mock/harness 广播（真机后端不发）"


def _rust_all() -> str:
    """src-tauri/src 全量 + qibai-core/src 全量。

    早期只读 main.rs，漏掉了 listener.rs（`pet-perform` / `ui-nav` / `listen-state` /
    `mute-toggle` / `tts-interrupted` 都在那里 emit），于是全被报成「后端从未 emit」。
    """
    text = ''
    for root in (os.path.join(ROOT, 'src-tauri', 'src'), os.path.join(ROOT, 'qibai-core', 'src')):
        for dp, _, fs in os.walk(root):
            for f in fs:
                if f.endswith('.rs'):
                    text += read(os.path.join(dp, f)) + '\n'
    return text


def _mock_broadcasts() -> set:
    names = set()
    for f in ('smoke.html', 'tauri-bridge.js'):
        path = os.path.join(SRC, f)
        if not os.path.exists(path):
            continue
        for m in re.finditer(r"broadcast\(\s*['\"]([^'\"]+)['\"]", read(path)):
            names.add(m.group(1))
    return names


def audit_events():
    """返回 (issues, notes)。issues=后端确实没有 emit 且也不是 mock 专用。"""
    issues, notes = [], []
    txt = _rust_all()
    emitted = set(re.findall(r'"((?:qibai|task)://[A-Za-z0-9_\-]+)"', txt))
    # 不带前缀的 emit 名（如 `emit("tts-playback", ...)`）也要按 qibai:// 归一化登记。
    for m in re.finditer(r"\.emit\(\s*['\"]([A-Za-z0-9_\-]+)['\"]", txt):
        name = m.group(1)
        if '://' not in name:
            emitted.add('qibai://' + name)
    # 变量式 emit：main.rs 把事件名放进 `(event, payload)` 再 `emit(event, ..)`，
    # 正则抓不到调用点的字面量，只能按源码里真实存在的字面量登记（仍然可复核）。
    for name in ('tts-playback', 'tts-interrupted'):
        if f'"{name}"' in txt:
            emitted.add('qibai://' + name)
    fe = ''
    for f in os.listdir(SRC):
        if f.endswith('.js'):
            fe += read(os.path.join(SRC, f))
    listens = {
        m.group(1)
        for m in re.finditer(r"listen\(\s*['\"]((?:qibai|task)://[^'\"]+)['\"]", fe)
    }
    mock = _mock_broadcasts()
    for e in sorted(listens):
        if e in emitted:
            continue
        if e in mock:
            notes.append(f"{e} {MOCK_ONLY_NOTE}")
            continue
        issues.append(f"前端监听 {e}，后端从未 emit")
    return issues, notes

def main():
    cmds, reg = parse_commands()
    calls = frontend_calls()
    # 标注注释里的调用
    for c in calls:
        path = os.path.join(SRC, c['file']); text = read(path)
        line_text = text.split('\n')[c['line'] - 1]
        if line_text.strip().startswith('*') or line_text.strip().startswith('//'):
            c['in_comment'] = True
    ci, cn = audit_contract(cmds, reg, calls)
    di, dn = audit_dom()
    ei, en = audit_events()
    print(f"命令 {len(cmds)} 个 / 注册 {len(reg)} 个 / 前端调用 {len(calls)} 处")
    for title, arr in [('命令契约', ci), ('DOM 与按钮绑定', di), ('事件名', ei)]:
        print(f"\n【{title}】{len(arr)} 处")
        for x in arr: print('  -', x)
    if cn or dn or en:
        print("\n【跳过/待人工确认】")
        for x in cn + dn + en: print('  ?', x)
    strict = '--strict' in sys.argv
    sys.exit(1 if ci or di or ei or (strict and (cn or dn)) else 0)

if __name__ == '__main__':
    main()
