#!/usr/bin/env bash
# com.qibai.websearch 占位执行体。
# 说明：web_search 是**内置技能**（BuiltinKind::WebSearch），运行时由
# qibai-core 的 skills.rs dispatch 直接拦截执行（搜狗→360→移动百度降级链），
# 本脚本永远不会被真正调用——它只用于插件清单格式演示。
echo "内置技能执行：web_search（本占位脚本不参与运行时）"
