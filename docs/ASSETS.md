# 美术素材说明与授权 / Assets

## 一、素材范围

| 类别 | 内容 | 存放位置（本地） | 是否在仓库内 |
|---|---|---|---|
| 角色立绘 / 动作帧 | 5 个角色，共 204 张 PNG | `src/assets/personas/<角色>/` | ❌ 不在仓库（631 MB） |
| 预览图 | 5 张单角色全身图 + 1 张五角色横排，合计约 550 KB | `docs/images/` | ✅ 在仓库 |
| 3D / AI 辅助小图 | 头像、图标、界面点缀 | `src/assets/ai_char*`、`src/assets/3d` | ✅ 在仓库（合计约 20 MB 以内） |
| 应用图标 | 三平台图标 | `src-tauri/icons/` | ✅ 在仓库 |

**仓库内已有的预览文件**（由真实立绘缩放而来，供 README 与文档引用）：

```
docs/images/personas/gudian.png   ← 古典佳人（默认角色）
docs/images/personas/dushi.png    ← 都市女性
docs/images/personas/wenrou.png   ← 温柔
docs/images/personas/huopo.png    ← 活泼
docs/images/personas/reqing.png   ← 热情时尚
docs/images/personas-all.png      ← 五角色横排，README 展示用
```

## 二、为什么全量帧不放仓库

204 张 PNG 合计 **631 MB**：

| 角色 | 体积 |
|---|---:|
| 温柔 `wenrou` | 249 MB |
| 活泼 `huopo` | 101 MB |
| 古典 `gudian` | 101 MB |
| 热情 `reqing` | 95 MB |
| 都市 `dushi` | 87 MB |

放进普通 git 会让任何一次 `git clone` 拉 600 MB+，仓库永久臃肿；放 Git LFS 会吃掉免费 1 GB 配额。因此：

- **仓库只放预览图**（`docs/images/personas/`，合计 < 2 MB）；
- **全量帧作为 GitHub Release 附件**分发（`personas_frames_v<版本>.zip`，必要时分卷）；
- 用 `fetch_personas.sh` 一键拉取到 `src/assets/personas/`。

## 三、获取全量帧

```bash
# 从最新 Release 拉取（脚本随仓库提供）
bash fetch_personas.sh

# 或手动：下载 Release 里的 personas_frames_*.zip 后
unzip personas_frames_v0.1.0.zip -d src/assets/personas/
```

目录必须保持 `<角色>/<动作>/<帧>.png` 结构，否则前端加载不到图。

## 四、授权（**重要，与代码的 MIT 分开**）

代码是 MIT，**素材不是**。建议按下表声明，你可以替换成自己需要的条款：

| 用途 | 是否允许 | 条件 |
|---|---|---|
| 个人学习、本地运行、修改自用 | ✅ 允许 | 保留出处 |
| fork 后非商业分发（含修改） | ✅ 允许 | 保留出处 + 同样条款（CC BY-NC-SA 4.0） |
| 商用（含商用产品内置、付费分发、广告素材） | ❌ 需授权 | 联系作者单独授权 |
| 用于训练生成模型 / 作为 AI 训练数据 | ❌ 禁止 | — |
| 二次销售素材本体 | ❌ 禁止 | — |

**推荐写法（可直接粘进 README 或本文件）**：

> 本项目美术素材（立绘、动作帧、表情、图标）由作者使用生成式模型创作，版权归作者所有，
> 以 **CC BY-NC-SA 4.0** 授权非商业使用；商业使用需获得作者书面许可。
> 素材中不含第三方版权内容、不含水印文字。

## 五、素材来源声明

- 全部由生成式模型产出，作者持有创作与使用权；
- 不含真实人物肖像、不含第三方 IP 角色、不含影视/游戏素材；
- 不含水印与签名文字。

## 六、注意

若你后续替换了生成模型或引入了外部素材，需同步更新本节与 `THIRD-PARTY-NOTICES.md`，
避免"代码 MIT 却夹带来源不明素材"的许可冲突。
