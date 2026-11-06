#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""浏览器冒烟截图：pet(huopo/gudian) + console + panel + guard。"""
import time
from playwright.sync_api import sync_playwright

import os
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, 'artifacts', 'ui_rework')
BASE = 'http://localhost:8765'

SHOTS = [
    ('pet_huopo', '/pet.html?persona=huopo', None),
    ('pet_gudian', '/pet.html?persona=gudian', None),
    ('pet_huopo_darkbg', '/pet.html?persona=huopo', 'dark'),   # 注入深色背景验白边
    ('pet_huopo_menu', '/pet.html?persona=huopo#fire=menu', None),  # 右键菜单唤出
    ('console', '/console.html', None),
    ('panel', '/panel.html', None),
    ('guard', '/guard.html', None),
]

with sync_playwright() as p:
    b = p.chromium.launch(executable_path='/usr/local/bin/chromium', args=['--no-sandbox'])
    pg = b.new_page(viewport={'width': 900, 'height': 900})
    for name, path, mode in SHOTS:
        pg.goto(BASE + path)
        pg.wait_for_timeout(400)
        pg.reload()  # 防 hash 同源导航不刷新
        pg.wait_for_timeout(1200)
        if mode == 'dark':
            pg.evaluate("document.body.style.background='#2d2d3c'")
            pg.wait_for_timeout(300)
        pg.screenshot(path=f'{OUT}/smoke_{name}.png')
        print('shot:', name)
    b.close()
print('done')
