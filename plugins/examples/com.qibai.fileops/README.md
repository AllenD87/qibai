# com.qibai.fileops

示例文件/文件夹操作技能插件（格式演示包）。

## 用法

对元气小小说：

- 「看看我主目录里有什么」→ `list`
- 「打开 ~/Downloads 文件夹」→ `open`
- 「在 ~/note.txt 新建一个笔记，内容是……」→ `create`
- 「把 ~/a.txt 重命名成 ~/b.txt」→ `rename`
- 「把 ~/report.txt 复制到 ~/backup/」→ `copy`
- 「删掉 ~/temp.txt」→ `delete`（**会先弹审批，确认后才删**）

## 安全设计（本地优先红线）

1. **路径限 $HOME**：`~` 展开后必须落在用户主目录内，越界一律拒绝。
2. **`..` 穿越拦截**：逐段路径重建，命中 `..` 直接拒绝。
3. **符号链接逃逸复查**：canonicalize 后二次确认仍在 $HOME 内。
4. **系统目录黑名单**：`/etc /usr /boot /var /proc /sys /dev /root` 拒绝操作。
5. **删除走审批**：`delete` 不直接执行，挂起 `qibai://skill-approval`
   事件等用户确认；批准后内核二次消毒才真正删除。
6. **主目录根保护**：`~` 根目录本身只允许 list/open，禁止 delete/rename。
7. **不覆盖**：create/move/copy 目标已存在时拒绝，不静默覆盖。

## 权限声明

`files:read` / `files:write`——内置信任包，无需用户逐项授权；
外部插件永不授予 `files:write`（注册层硬拒）。

## 安装

```bash
cp -r com.qibai.fileops <plugins_dir>/
```
