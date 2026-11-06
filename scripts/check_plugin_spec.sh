#!/usr/bin/env bash
# ============================================================
# 《元气小小》插件双规范自动校验器（PLUGIN_V18_SPEC 沉淀）
# 用法: bash scripts/check_plugin_spec.sh [plugins_dir]
# 默认扫描 src-tauri/plugins；输出 PASS/FAIL 汇总，任一 FAIL 退出码 1。
# 校验项（与 docs/PLUGIN_V18_SPEC.md 逐条对应）：
#   S1 每个包 qibai-plugin.json 存在且 JSON 合法
#   S2 id 前缀/全小写/全局唯一
#   S3 skill: type=skill + 权限最小集 + entry.command=run.sh + run.sh 语法
#   S4 expert: type=expert + system_prompt 非空
#   S5 WorkBuddy 专家: .codebuddy-plugin/plugin.json 合法 + displayDescription.zh
#      中文 40~50 字 + tags 恰 3 + quickPrompts 非空 + agents 引用文件存在
#   S6 红线: run.sh 无 rm -rf 根/家目录 / 硬编码密钥 / 危险命令（code-review
#      等扫描器仅豁免其模式串定义行）
# 历史包（v18 前：code-check/file-locate/code-expert/plan-expert）只做
# S1/S2 兼容检查，不套 v18 结构硬要求。
# ============================================================
set -u
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PLUGINS="${1:-$ROOT/src-tauri/plugins}"
FAILS=0; CHECKS=0
say()  { printf '%s\n' "$*"; }
fail() { say "FAIL  $*"; FAILS=$((FAILS+1)); }

PY=$(command -v python3 || echo "")
[ -z "$PY" ] && { say "需要 python3（JSON/字数校验）"; exit 2; }

LEGACY="code-check file-locate code-expert plan-expert"

# 收集全部清单 id，供去重
IDS=$("$PY" - "$PLUGINS" <<'EOF'
import json,sys,os
root=sys.argv[1]
out=[]
for kind in ("skills","experts"):
    d=os.path.join(root,kind)
    if not os.path.isdir(d): continue
    for name in sorted(os.listdir(d)):
        p=os.path.join(d,name,"qibai-plugin.json")
        if os.path.isfile(p):
            try:
                j=json.load(open(p,encoding="utf-8"))
                out.append((kind,name,j.get("id",""),j.get("type","")))
            except Exception as e:
                out.append((kind,name,"JSON_ERR:"+str(e),""))
for row in out: print("\t".join(row))
EOF
)

say "== 校验插件库 $PLUGINS =="
DUP=$(printf '%s\n' "$IDS" | cut -f3 | sort | uniq -d | grep -v '^$' || true)
if [ -n "$DUP" ]; then fail "id 重复: $DUP"; else say "PASS  id 全局唯一"; CHECKS=$((CHECKS+1)); fi

while IFS=$'\t' read -r kind name id typ; do
  [ -z "$name" ] && continue
  p="$PLUGINS/$kind/$name"
  is_legacy=0; for L in $LEGACY; do [ "$name" = "$L" ] && is_legacy=1; done
  [ -f "$p/qibai-plugin.json" ] || { fail "[$name] 缺 qibai-plugin.json"; continue; }
  if ! "$PY" -c "import json,sys;json.load(open(sys.argv[1]))" "$p/qibai-plugin.json" 2>/dev/null; then
    fail "[$name] qibai-plugin.json JSON 非法"; continue
  fi
  CHECKS=$((CHECKS+1)); say "PASS  [$name] 清单 JSON 合法"
  if [ "$kind" = skills ] && ! printf '%s' "$id" | grep -qE '^com\.qibai\.skill\.[a-z0-9]+$'; then fail "[$name] id 非法: $id"; fi
  if [ "$kind" = experts ] && ! printf '%s' "$id" | grep -qE '^com\.qibai\.expert\.[a-z0-9]+$'; then fail "[$name] id 非法: $id"; fi
  [ "$is_legacy" = 1 ] && continue   # 历史包仅兼容检查
  if [ "$kind" = skills ] && [ "$typ" != skill ]; then fail "[$name] type 应为 skill"; fi
  if [ "$kind" = experts ] && [ "$typ" != expert ]; then fail "[$name] type 应为 expert"; fi
  # skill 专属：run.sh 存在且语法通过
  if [ "$kind" = skills ]; then
    if [ -f "$p/run.sh" ]; then
      if bash -n "$p/run.sh" 2>/dev/null; then CHECKS=$((CHECKS+1)); else fail "[$name] run.sh 语法错误"; fi
    else
      fail "[$name] 缺 run.sh"
    fi
    [ -f "$p/SKILL.md" ] || fail "[$name] 缺 SKILL.md"
  fi
done < <(printf '%s\n' "$IDS" | grep -v '^$')

# S5/S6 深度校验
"$PY" - "$PLUGINS" "$LEGACY" <<'EOF' | while IFS= read -r line; do case "$line" in PASS*) say "$line"; CHECKS=$((CHECKS+1));; FAIL*) fail "${line#FAIL  }";; esac; done
import json,os,re,sys
root,legacy=sys.argv[1],sys.argv[2].split()
def cjk(s): return len([c for c in s if '\u4e00'<=c<='\u9fff'])
def in_legacy(name): return name in legacy
for kind in ("skills","experts"):
    d=os.path.join(root,kind)
    if not os.path.isdir(d): continue
    for name in sorted(os.listdir(d)):
        p=os.path.join(d,name)
        jf=os.path.join(p,"qibai-plugin.json")
        if not os.path.isfile(jf): continue
        if in_legacy(name): continue
        j=json.load(open(jf,encoding="utf-8"))
        if kind=="skills":
            perm=set(j.get("permissions",[]) or [])
            ok=set(["files:read","network:read","reminder:set"])
            if not perm<=ok:
                print("FAIL  [%s] 权限越界: %s"%(name,sorted(perm)))
            ent=j.get("entry",{})
            if ent.get("kind")!="command" or ent.get("command")!="run.sh":
                print("FAIL  [%s] entry 应 command:run.sh"%name)
        if kind=="experts":
            if not (j.get("system_prompt") or "").strip():
                print("FAIL  [%s] system_prompt 为空"%name)
            wb=os.path.join(p,".codebuddy-plugin","plugin.json")
            if os.path.isfile(wb):
                w=json.load(open(wb,encoding="utf-8"))
                dd=w.get("displayDescription",{})
                zh=dd.get("zh","") if isinstance(dd,dict) else str(dd)
                n=cjk(zh)
                if not (40<=n<=50):
                    print("FAIL  [%s] displayDescription.zh 中文 %d 字（需40~50）"%(name,n))
                if len(w.get("tags",[]) or [])!=3:
                    print("FAIL  [%s] tags 应为 3 个"%name)
                if not (w.get("quickPrompts") or []):
                    print("FAIL  [%s] quickPrompts 缺失"%name)
                for ag in w.get("agents",[]) or []:
                    ap=os.path.join(p,ag) if not os.path.isabs(ag) else ag
                    if not os.path.isfile(ap):
                        print("FAIL  [%s] agents 引用缺失: %s"%(name,ag))
            else:
                print("FAIL  [%s] 缺 .codebuddy-plugin/plugin.json"%name)
# S6 红线扫描（code-review 豁免其模式串定义）
for kind in ("skills","experts"):
    d=os.path.join(root,kind)
    if not os.path.isdir(d): continue
    for name in sorted(os.listdir(d)):
        p=os.path.join(d,name)
        if in_legacy(name): continue
        for sh in ("run.sh","entry.sh"):
            f=os.path.join(p,sh)
            if not os.path.isfile(f): continue
            lines=open(f,encoding="utf-8",errors="ignore").read().splitlines()
            for i,ln in enumerate(lines):
                t=re.sub(r"#.*","",ln)
                if re.search(r"\brm\s+-rf\s*/[\"' ]",t) or re.search(r"\brm\s+-rf\b.*\$\{?HOME\}",t):
                    if name=="code-review" and ("rm -rf" in ln and re.search(r'scan\s+"',ln)):
                        continue  # 扫描器模式串定义，非真实调用
                    print("FAIL  [%s] %s:%d 危险 rm -rf"%(name,sh,i+1))
                if re.search(r"(ak|api[_-]?key|token|secret)\s*[=:]\s*['\"]?(sk-|ak-|[A-Za-z0-9]{20,})",t,re.I):
                    if name=="code-review" and ("scan" in ln or "grep" in ln):
                        continue  # 扫描模式定义
                    print("FAIL  [%s] %s:%d 疑似硬编码密钥"%(name,sh,i+1))
print("PASS  深度校验完成（权限/专家WB配置/红线扫描）")
EOF

say "== 结果: $CHECKS 项 PASS，$FAILS 项 FAIL =="
[ "$FAILS" -eq 0 ] || exit 1
exit 0
