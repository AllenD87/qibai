#!/usr/bin/env bash
# com.qibai.datetime 执行体
# 入参约定：LLM 调用参数 args 序列化为单行 JSON，经 stdin 传入（本示例不读取也无所谓）。
# 出参：stdout 文本回填给 LLM。
input="$(cat)"
date "+%Y-%m-%d %H:%M:%S"
