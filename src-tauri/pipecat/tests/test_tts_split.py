# -*- coding: utf-8 -*-
"""中文切句单测。"""
from pipecat_voice.tts import split_sentences


def test_mixed_punctuation():
    out = split_sentences("你好呀！今天开心吗？一起散步吧。")
    assert out == ["你好呀！", "今天开心吗？", "一起散步吧。"]


def test_no_punct_single_chunk():
    assert split_sentences("你好") == ["你好"]


def test_long_soft_cut():
    # 缓冲超 48 字仍无句末标点 → 遇软标点强制切
    text = "这是一句很长很长很长很长很长很长的中文回复需要在中间某个逗号位置被切开不然合成会太久影响开口速度啊，尾巴"
    out = split_sentences(text)
    assert len(out) >= 2
    assert sum(len(s) for s in out) == len(text)


def test_empty():
    assert split_sentences("") == []
    assert split_sentences("   ") == []


def test_ellipsis():
    out = split_sentences("嗯……我想想。好吧")
    assert out == ["嗯……", "我想想。", "好吧"]
