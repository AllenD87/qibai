# 对标 Remotion：分工与超越点

结论先说：**不是复刻 Remotion**。它强在 React 生态、云端分发、成熟文档；本技能补的是它没覆盖的空位——**生成结果的质量自证、零依赖离线、多比例一次成型**。

## 事实对照

| 维度 | Remotion | animacode v2 |
|---|---|---|
| 编程模型 | React 组件 + hooks，`useCurrentFrame` 驱动 | 声明式 `scene.json` + 八个镜头块；也可手写 canvas 图层 |
| 依赖 | npm 工程、Node、React、bundler | 零 npm 依赖；Chromium + Node + ffmpeg 即可 |
| 预览 | Studio（需 `npx remotion studio`） | 本地 Studio（`preview.mjs` 直接起） |
| 多比例 | 需自行写多套 composition 或按 props 分支 | 一份 spec 出多版，版式按 `H>W` 自动重排 |
| 渲染产物质量校验 | 无内建门禁，靠人眼/自建测试 | **内建**：安全区、重叠、小字号、对比度、空帧、长静止、突变、视觉回归 |
| 审计可视化 | 无 | Studio 叠加审计框（安全区边线 + 每个文字盒） |
| 许可 | 个人/≤3 人/非营利免费；大公司 Creators $25/席/月；自动化产品 $0.01/次、$100/月起；5.0 起 Automators 强制 licenseKey + 遥测 | 无许可服务、无遥测，可私有化 |
| 音频 | `@remotion/media-utils` 等，需额外依赖 | 场景内 `audio` 字段 + ffmpeg 混流，无额外包 |
| 云端分发 | Lambda / Cloud Run / 自建 | 本地即可，分发不是目标 |

Remotion 的价格与许可条款随版本变化，以官网为准：`/docs/license/pricing`、`/docs/license/faq`、`/docs/terms`、`/docs/telemetry`。

## 六条真实的超越点（有产物作证）

1. **质量门禁**：渲完即出 `qa_report.json`，`PASS / PASS_WITH_WARNINGS / FAIL` 可判定。v2 两支成片实测：9:16 全 `PASS`（overlap 0 / outside-safe 0 / tiny 0 / contrast 0），16:9 仅一条尾卡 0.6s 有意停留。Remotion 没有对应能力，模型写完只能靠人看。
2. **对比度门禁不误报**：环形取样 + `alpha<0.9` 稳定态过滤 + 按字号分档，解决"淡入文字被当成低对比"的假阳性。
3. **视觉回归指纹**：16×16 灰度哈希 + 基线比对，改一处能证明没带歪别处；`--fail-on-change` 可进 CI。
4. **一 spec 多格式**：同一份 `data` 出 1080×1920 与 1920×1080，版式自动重排（流程竖排、条形图重算留白、字号按 `u` 缩放）。Remotion 要写两套。
5. **零依赖离线**：无 npm、不出网、无遥测、无许可校验，断网内网可跑完整链路；这是私有化部署的硬门槛，Remotion 私有化可行但许可不免费。
6. **审计可视化 Studio**：画布上直接看安全区与每个文字盒，告警带时间戳可点跳帧——把"质检报告"和"画面"对到一起。

## 明确不追的部分

- **React 生态与组件复用**：不追。需要复杂交互式 composition 时用 Remotion 更合适。
- **云端弹性渲染**：不追。要的是单机确定性，不是横向扩容。
- **三方包生态**：不追。零依赖是设计约束，不是缺点。

## 一句话定位

Remotion 解决"能不能用代码写动画"；animacode 解决"写完怎么证明它够好、能不能在断网机器上稳定出多版"。
