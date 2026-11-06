# 美术素材说明与授权 / Assets

## 一、素材范围

| 类别 | 内容 | 存放位置（本地） | 是否在仓库内 |
|---|---|---|---|
| 角色立绘 / 动作帧 | 5 个角色，共 120 张 PNG | `src/assets/personas/<角色>/` | ❌ 不在仓库（478 MB） |
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

120 张 PNG 合计 **478 MB**：

| 角色 | 体积 | 帧数 |
|---|---:|---:|
| 古典 `gudian` | 101 MB | 24 |
| 活泼 `huopo` | 101 MB | 24 |
| 温柔 `wenrou` | 96 MB | 24 |
| 热情 `reqing` | 95 MB | 24 |
| 都市 `dushi` | 87 MB | 24 |

放进普通 git 会让任何一次 `git clone` 拉近 500 MB，仓库永久臃肿；放 Git LFS 会吃掉免费 1 GB 配额。因此：

- **仓库只放预览图**（`docs/images/personas/`，合计 < 2 MB）；
- **全量帧作为 GitHub Release 附件**分发，按 160 MB 分卷（GitHub 单文件上限 2 GB，分卷主要是为了断点续传与失败重试）；
- 用 `fetch_personas.sh` 一键拉取、校验、解压到 `src/assets/personas/`。

## 三、获取全量帧

```bash
# 一条命令搞定：下载 → 逐卷 md5 校验 → 拼接 → 整包校验 → 解压（实测 54 秒）
bash fetch_personas.sh

# 已装过、想重下覆盖
bash fetch_personas.sh --force

# 换版本
QIBAI_PERSONAS_TAG=v0.2.0 bash fetch_personas.sh
```

脚本需要 `curl` + `unzip`，md5 优先用 `md5sum`（Linux）/ `md5`（macOS），都没有时跳过校验并提示。

**手动方式**（万一脚本跑不通）：

```bash
BASE=https://github.com/AllenD87/qibai/releases/download/v0.1.0/personas_frames_v0.1.0.zip
for i in 0 1 2; do curl -fL -o part$i "$BASE.part$i"; done
cat part0 part1 part2 > personas_frames_v0.1.0.zip
md5sum personas_frames_v0.1.0.zip    # 应为 e6aafa6ff36334c15b7cb07c921510f2
unzip personas_frames_v0.1.0.zip -d src/assets
```

当前版本 `v0.1.0` 的三个分卷（各卷 md5 供排错用）：

| 分卷 | 大小 | md5 |
|---|---:|---|
| `personas_frames_v0.1.0.zip.part0` | 160 MB | `490606a16d885d76c5e9cf6ac929838f` |
| `personas_frames_v0.1.0.zip.part1` | 160 MB | `c9aca441173567297fc1e629b195a9bb` |
| `personas_frames_v0.1.0.zip.part2` | 157 MB | `1bf02aa4abb6cdcbbe994396c900c9a6` |

目录必须保持 `<角色>/<动作>/<帧>.png` 结构（`poses/`、`expressions/`），否则前端按
`assets/personas/<角色>/<动作>/<帧>.png` 相对路径加载不到图。

> 注意：作者工作目录里的 `personas/wenrou/` 下曾混入过另外四个角色的**旧版帧副本**
> （`wenrou/{gudian,dushi,huopo,reqing}/`，约 165 MB，与实际角色目录的帧不一致）。
> 那是早期批量生成时工作目录写错留下的，应用不会按这个路径取图，所以**发布包已排除它们**，
> 只保留五个角色的正式目录。如果你手上是原始工程目录，可以自行删掉这四个嵌套目录省 165 MB。

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
