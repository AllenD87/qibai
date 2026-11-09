//! Animacode 动画连接器的「真能调用」验收测试。
//!
//! 走的是产品里的真实链路：清单加载（`{PLUGIN_DIR}` 变量替换）→ `SkillAdapter`
//! 按 `entry.kind=mcp` 拉起 stdio server → JSON-RPC 握手/tools/list → 真跑工具。
//! 依赖 node；缺 node 的机器上本用例跳过（打印 SKIP，不算失败）。

use qibai_core::plugin_manifest::ManifestLoader;
use qibai_core::skill_adapter::SkillAdapter;
use qibai_core::skills::SkillOutcome;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::time::Duration;

fn copy_dir(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for e in std::fs::read_dir(src).unwrap().flatten() {
        let p = e.path();
        let t = dst.join(e.file_name());
        if p.is_dir() {
            copy_dir(&p, &t);
        } else {
            std::fs::copy(&p, &t).unwrap();
        }
    }
}

/// 路径比较用：统一分隔符、去掉 Windows `\\?\` 前缀（canonicalize 会加）。
fn norm(p: &std::path::Path) -> String {
    let s = p.display().to_string().replace('\\', "/");
    s.strip_prefix("//?/").unwrap_or(&s).to_string()
}

fn node_available() -> bool {
    std::process::Command::new("node")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    }

/// 把仓库里的 animacode 包复制成一个「安装好的用户插件」，返回（安装根, 包目录）。
fn stage_package(tag: &str) -> (PathBuf, PathBuf) {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/plugins/animacode");
    let root = std::env::temp_dir().join(format!("qibai_anima_{tag}_{}", std::process::id()));
    let pkg = root.join("plugins").join("animacode");
    let _ = std::fs::remove_dir_all(&root);
    copy_dir(&src, &pkg);
    (root, pkg)
}

#[tokio::test(flavor = "multi_thread")]
async fn mcp_connector_runs_schema_and_build() {
    if !node_available() {
        eprintln!("SKIP: 本机没有 node，跳过 animacode MCP 调用验收");
        return;
    }
    let (root, pkg) = stage_package("main");

    let manifest = ManifestLoader::load(&pkg).expect("animacode 清单应能加载");
    assert_eq!(manifest.id, "animacode");
    assert_eq!(manifest.functions.len(), 5, "应暴露五个 anima_* 工具");
    let args_text = manifest
        .entry
        .as_ref()
        .and_then(|e| e.mcp.as_ref())
        .map(|m| m.args.join(" "))
        .expect("mcp 入口应存在");
    assert!(
        !args_text.contains("{PLUGIN_DIR}") && !args_text.contains("{QIBAI_HOME}"),
        "变量应已被替换：{args_text}"
    );
    assert_eq!(
        norm(Path::new(&args_text)),
        norm(&pkg.canonicalize().unwrap_or(pkg.clone()).join("mcp/server.mjs")),
        "入口路径应落在包目录内"
    );

    let adapter = SkillAdapter::with_timeout(Duration::from_secs(60));

    // 0a) 技能包导入路径（设置页「导入技能包」走的就是这个函数）
    {
        let pack_zip = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../examples/plugins/animacode-v2.1.0.qbpk");
        let plugdir = root.join("install_plugins");
        let installed = qibai_core::skill_pack::install_pack(&pack_zip, &plugdir)
            .expect("技能包应能导入");
        assert_eq!(installed.id, "animacode");
        let args_text = installed
            .entry
            .as_ref()
            .and_then(|e| e.mcp.as_ref())
            .map(|m| m.args.join(" "))
            .unwrap();
        let installed_dir = plugdir
            .join("animacode")
            .canonicalize()
            .unwrap_or(plugdir.join("animacode"));
        assert_eq!(
            norm(Path::new(&args_text)),
            norm(&installed_dir.join("mcp/server.mjs")),
            "导入后的入口应落在安装目录内"
        );
    }

    // 0) 产品侧发现链路：注册表扫描到包 → 启用后进 LLM 工具表
    {
        std::env::set_var("QIBAI_PLUGINS_DIR", root.join("plugins"));
        let reg = qibai_core::plugin_registry::PluginRegistry::new(
            &qibai_core::settings::PluginSettings::default(),
            &qibai_core::settings::SkillsSettings::default(),
        );
        let entry = reg.entry("animacode").expect("注册表应扫到 animacode");
        assert!(entry.error.is_none(), "清单应无错误：{:?}", entry.error);
        let before: Vec<String> = reg
            .tool_schemas()
            .iter()
            .filter_map(|s| s["function"]["name"].as_str().map(|x| x.to_string()))
            .collect();
        assert!(
            !before.iter().any(|n| n.starts_with("anima_")),
            "外部包默认不启用，不应进工具表：{before:?}"
        );
        let mut reg = reg;
        reg.set_enabled("animacode", true).expect("启用应成功");
        let names: Vec<String> = reg
            .tool_schemas()
            .iter()
            .filter_map(|s| s["function"]["name"].as_str().map(|x| x.to_string()))
            .collect();
        for f in ["anima_schema", "anima_build", "anima_render", "anima_qa", "anima_mux"] {
            assert!(names.contains(&f.to_string()), "工具表缺 {f}：{names:?}");
        }
        std::env::remove_var("QIBAI_PLUGINS_DIR");
    }

    // 1) 抓手一：anima_schema（零参数，验证握手 + tools/list + 调用）
    match adapter.execute(&pkg, &manifest, "anima_schema", &json!({})).await {
        SkillOutcome::Ok(text) => {
            assert!(text.contains("duration") || text.contains("scene.json"),
                "schema 应返回字段说明，实际: {}", &text[..text.len().min(200)]);
        }
        other => panic!("anima_schema 调用应成功，实际 {other:?}"),
    }

    // 2) 抓手二：anima_build（真跑脚本，验证 spec 校验与多比例构建）
    let spec_src = pkg.join("assets/example-spec/scene.json");
    let spec = root.join("scene.json");
    std::fs::copy(&spec_src, &spec).unwrap();
    match adapter
        .execute(&pkg, &manifest, "anima_build", &json!({ "spec": spec.display().to_string() }))
        .await
    {
        SkillOutcome::Ok(text) => assert!(
            text.contains("built") && text.contains("169"),
            "构建应产出 169 比例，实际: {text}"
        ),
        other => panic!("anima_build 调用应成功，实际 {other:?}"),
    }

    let _ = std::fs::remove_dir_all(&root);
}
