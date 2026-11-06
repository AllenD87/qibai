//! 迭代④联调冒烟：最小真连流程（注册 → 连接 → 发消息 → 收 echo → 断开）。
//!
//! 运行：
//! ```bash
//! cargo run -p qibai-core --example netsocial_live -- http://127.0.0.1:8787
//! ```
//!
//! 服务端由并行 SubAgent 开发中（`node server/src/server.js`）；
//! 本阶段只要求编译通过，联调阶段在真服务端上显式运行本示例。
//! 数据目录指向临时目录，不污染真实数据。

use qibai_core::netsocial::NetsocialManager;

#[tokio::main]
async fn main() {
    let url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "http://127.0.0.1:8787".to_string());
    println!("[netsocial-live] 目标服务器：{url}");

    // 数据目录指向临时目录，不污染真实数据。
    let dir = std::env::temp_dir().join("qibai-netsocial-live");
    std::fs::create_dir_all(&dir).unwrap();
    // NetsocialManager::new() 在构造时解析数据目录，故先置环境变量。
    std::env::set_var("QIBAI_DATA_DIR", &dir);

    let mgr = NetsocialManager::new();

    // 1. 注册（本地生成匿名身份；无 server_url 时仅本地保存为离线占位）。
    let reg = mgr
        .register(&serde_json::json!({
            "nickname": "联调A",
            "color_idx": 3,
            "city": "石家庄",
            "age_declared": "18plus",
            "pure_mode": false,
            "avatar": 0,
            "soul_tags": {
                "personality": ["温柔"],
                "interests": ["音乐"],
                "status": ["想聊天"]
            }
        }))
        .await
        .expect("register 失败");
    println!("[netsocial-live] 注册完成：{reg}");

    // 2. 连接（HTTP 补注册换 token → WebSocket 握手 → Online）。
    mgr.connect(Some(url.clone()))
        .await
        .expect("connect 失败");
    println!("[netsocial-live] 已连接，状态：{}", mgr.status()["state"]);

    // 3. 漫游浏览（原样透传服务端 souls 列表）。
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    match mgr.roam(&serde_json::json!({"online": "true"})).await {
        Ok(souls) => println!("[netsocial-live] 漫游列表：{souls}"),
        Err(e) => println!("[netsocial-live] 漫游失败（服务端可能未就绪）：{e}"),
    }

    // 4. 轮询事件（connected / hello / inbox / msg echo / presence / error）。
    for i in 0..20 {
        let polled = mgr.poll();
        let events = polled["events"].as_array().cloned().unwrap_or_default();
        if !events.is_empty() {
            println!("[netsocial-live] 事件#{i}：{polled}");
        }
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }

    // 5. 断开（Online → Offline，停止后台重连任务）。
    mgr.disconnect().expect("disconnect 失败");
    println!("[netsocial-live] 已断开，状态：{}", mgr.status()["state"]);
}
