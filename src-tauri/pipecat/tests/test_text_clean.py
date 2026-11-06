# -*- coding: utf-8 -*-
"""朗读前文本清洗单测：markdown / 链接 / emoji / 特殊符号一律不读出来。"""
from pipecat_voice.tts import clean_for_tts, make_tts, split_sentences


def test_bold_italic_markers_removed():
    assert clean_for_tts("**加粗**和*斜体*都不要读出来") == "加粗和斜体都不要读出来"


def test_heading_hash_removed():
    assert clean_for_tts("# 标题一\n正文内容") == "标题一 正文内容"


def test_list_markers_removed():
    out = clean_for_tts("- 第一项\n- 第二项\n1. 第三项")
    assert out == "第一项 第二项 第三项"


def test_inline_code_backticks_removed():
    assert clean_for_tts("运行 `npm install` 即可") == "运行 npm install 即可"


def test_code_fence_removed():
    out = clean_for_tts("```python\nprint(1)\n```")
    assert "```" not in out
    assert "print(1)" in out


def test_markdown_link_keeps_text():
    assert clean_for_tts("看[百度](https://www.baidu.com)一下") == "看百度一下"


def test_emoji_stripped():
    out = clean_for_tts("你好呀🎉，今天开心吗😄")
    assert "🎉" not in out
    assert "😄" not in out
    assert "你好呀" in out
    assert "今天开心吗" in out


def test_whitespace_collapsed():
    assert clean_for_tts("你好   世界\t\n再来一句") == "你好 世界 再来一句"


def test_chinese_punctuation_kept():
    s = "你好，世界！这是「测试」：括号（）【】《》都要保留……"
    out = clean_for_tts(s)
    for ch in "，！：（）【】《》……":
        assert ch in out, f"标点 {ch} 被误删: {out}"


def test_plain_text_unchanged():
    s = "今天天气真不错，我们一起去散步吧。"
    assert clean_for_tts(s) == s


def test_empty():
    assert clean_for_tts("") == ""
    assert clean_for_tts(None) == ""  # type: ignore[arg-type]


def test_make_tts_wraps_cleaning():
    # make_tts("null") 返回的合成函数入口已清洗：输入带 markdown 也不报错、产出非空。
    fn = make_tts("null")
    pcm = fn("**你好**🎉")
    assert isinstance(pcm, bytes) and len(pcm) > 0


def test_clean_then_split_still_works():
    # 清洗后再分句：保证朗读自然、不把 markdown 标记切进句子。
    sents = split_sentences(clean_for_tts("# 标题\n第一句！第二句？"))
    assert sents == ["标题 第一句！", "第二句？"]
