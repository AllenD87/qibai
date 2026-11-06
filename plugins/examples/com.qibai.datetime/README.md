# com.qibai.datetime

示例 command-kind 技能插件（契约 §6）。

## stdin JSON 入参约定

LLM 调用 `get_datetime` 时，`args`（JSON 对象）会被序列化为**单行 JSON 文本**经
`stdin` 传入本脚本。例如：

```json
{"format":"%Y-%m-%d"}
```

脚本读取 stdin（`cat`）后执行，stdout 输出文本回填给 LLM。执行超时 10s；
超时/失败自动降级为 `技能执行失败（已降级）：{原因}`。

## 权限声明

`command:run`——安装后需经用户逐项授权（`plugin_grant_permission`）方可调用。

## 安装

```bash
# 复制到 plugins 目录，或经 IPC plugin_install 指向本目录
cp -r com.qibai.datetime <plugins_dir>/
```
