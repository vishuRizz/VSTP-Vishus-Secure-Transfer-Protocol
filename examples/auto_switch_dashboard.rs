use axum::{
    extract::State,
    response::{Html, IntoResponse},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::sync::Mutex;
use vstp::easy::{AutoFaultInjection, AutoSwitchConfig, VstpClient, VstpServer};

#[derive(Clone)]
struct AppState {
    client: Arc<Mutex<VstpClient>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct DemoRequest {
    message: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct DemoResponse {
    ok: bool,
    details: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let server_addr = "127.0.0.1:9301";

    let server = VstpServer::bind_auto(server_addr).await?;
    tokio::spawn(async move {
        let _ = server
            .serve(|msg: String| async move { Ok(format!("echo:{msg}")) })
            .await;
    });

    tokio::time::sleep(Duration::from_millis(150)).await;

    let cfg = AutoSwitchConfig {
        probe_attempts: 2,
        probe_timeout: Duration::from_millis(500),
        switch_cooldown: Duration::from_secs(2),
        min_dwell_time: Duration::from_secs(1),
        consecutive_failures_threshold: 2,
        min_score_margin_ms: 5.0,
        ack_timeout_penalty_ms: 25.0,
        io_error_penalty_ms: 35.0,
        peer_preference_ttl: Duration::from_secs(120),
    };
    let client = VstpClient::connect_auto_with_config(server_addr, cfg).await?;
    let client = Arc::new(Mutex::new(client));

    let worker_client = client.clone();
    tokio::spawn(async move {
        let mut i = 0u64;
        loop {
            i += 1;
            let payload = format!("tick-{i}");
            let guard = worker_client.lock().await;
            let _ = guard.send(payload.clone()).await;
            let _ = guard.receive::<String>().await;
            drop(guard);
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
    });

    let state = AppState { client };
    let app = Router::new()
        .route("/", get(index))
        .route("/api/status", get(status))
        .route("/api/faults", post(set_faults))
        .route("/api/ping", post(ping))
        .with_state(state);

    let web_addr: SocketAddr = "127.0.0.1:8088".parse()?;
    println!("Dashboard: http://{web_addr}");
    println!("Demo VSTP server: {server_addr}");

    let listener = tokio::net::TcpListener::bind(web_addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn index() -> impl IntoResponse {
    Html(
        r#"<!doctype html>
<html>
<head>
  <meta charset="utf-8" />
  <title>VSTP Auto Switch Dashboard</title>
  <style>
    body { font-family: sans-serif; max-width: 900px; margin: 2rem auto; }
    .row { display: flex; gap: 1rem; margin-bottom: .8rem; }
    label { min-width: 220px; }
    input { width: 150px; }
    pre { background: #f5f5f5; padding: 1rem; border-radius: 8px; }
    button { padding: .5rem .9rem; }
  </style>
</head>
<body>
  <h2>VSTP Auto Transport Switch</h2>
  <p>Change these factors and watch active transport switch in real time.</p>
  <div class="row"><label>TCP Delay (ms)</label><input id="tcp_delay_ms" type="number" value="0" /></div>
  <div class="row"><label>UDP Delay (ms)</label><input id="udp_delay_ms" type="number" value="0" /></div>
  <div class="row"><label>TCP Fail Every N ops</label><input id="tcp_fail_every_n" type="number" value="0" /></div>
  <div class="row"><label>UDP Fail Every N ops</label><input id="udp_fail_every_n" type="number" value="0" /></div>
  <button onclick="applyFaults()">Apply Factors</button>
  <button onclick="sendPing()">Send Ping</button>
  <h3>Live Status</h3>
  <pre id="status"></pre>

  <script>
    async function refresh() {
      const r = await fetch('/api/status');
      const j = await r.json();
      document.getElementById('status').textContent = JSON.stringify(j, null, 2);
    }
    async function applyFaults() {
      const payload = {
        tcp_delay_ms: Number(document.getElementById('tcp_delay_ms').value || 0),
        udp_delay_ms: Number(document.getElementById('udp_delay_ms').value || 0),
        tcp_fail_every_n: Number(document.getElementById('tcp_fail_every_n').value || 0),
        udp_fail_every_n: Number(document.getElementById('udp_fail_every_n').value || 0)
      };
      await fetch('/api/faults', { method: 'POST', headers: {'content-type':'application/json'}, body: JSON.stringify(payload) });
      await refresh();
    }
    async function sendPing() {
      await fetch('/api/ping', { method: 'POST', headers: {'content-type':'application/json'}, body: JSON.stringify({message: 'manual'}) });
      await refresh();
    }
    setInterval(refresh, 1000);
    refresh();
  </script>
</body>
</html>"#,
    )
}

async fn status(State(state): State<AppState>) -> impl IntoResponse {
    let guard = state.client.lock().await;
    match guard.auto_status().await {
        Ok(s) => Json(serde_json::json!({ "ok": true, "status": s })),
        Err(e) => Json(serde_json::json!({ "ok": false, "error": e.to_string() })),
    }
}

async fn set_faults(
    State(state): State<AppState>,
    Json(faults): Json<AutoFaultInjection>,
) -> impl IntoResponse {
    let guard = state.client.lock().await;
    match guard.set_auto_fault_injection(faults).await {
        Ok(_) => Json(DemoResponse {
            ok: true,
            details: "faults updated".to_string(),
        }),
        Err(e) => Json(DemoResponse {
            ok: false,
            details: e.to_string(),
        }),
    }
}

async fn ping(State(state): State<AppState>, Json(req): Json<DemoRequest>) -> impl IntoResponse {
    let guard = state.client.lock().await;
    let send = guard.send(req.message).await;
    let recv = guard.receive::<String>().await;
    match (send, recv) {
        (Ok(_), Ok(msg)) => Json(DemoResponse {
            ok: true,
            details: msg,
        }),
        (Err(e), _) => Json(DemoResponse {
            ok: false,
            details: e.to_string(),
        }),
        (_, Err(e)) => Json(DemoResponse {
            ok: false,
            details: e.to_string(),
        }),
    }
}
