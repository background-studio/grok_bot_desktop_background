use std::{
    net::{TcpListener, TcpStream},
    os::windows::process::CommandExt,
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use serde::Deserialize;
use serde_json::{json, Value};
use tungstenite::{connect, stream::MaybeTlsStream, Message, WebSocket};
use url::Url;

use crate::injector::{native_titlebar_bridge_ready, read_browser_identity};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const PREFERRED_INSPECTOR_PORT: u16 = 9238;
const INSPECTOR_WAIT: Duration = Duration::from_secs(15);
const RENDERER_WAIT: Duration = Duration::from_secs(45);

const WCO_PATCH: &str = include_str!("../../src/main/native-titlebar.cjs");

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InspectorTarget {
    #[serde(rename = "type")]
    target_type: String,
    web_socket_debugger_url: String,
}

type InspectorSocket = WebSocket<MaybeTlsStream<TcpStream>>;

struct InspectorSession {
    socket: InspectorSocket,
    next_id: u64,
}

impl InspectorSession {
    fn open(target: &InspectorTarget, port: u16) -> Result<Self, String> {
        let websocket = validate_inspector_websocket(&target.web_socket_debugger_url, port)?;
        let (mut socket, _) = connect(websocket.as_str()).map_err(|error| error.to_string())?;
        if let MaybeTlsStream::Plain(stream) = socket.get_mut() {
            stream
                .set_read_timeout(Some(Duration::from_secs(20)))
                .map_err(|error| error.to_string())?;
            stream
                .set_write_timeout(Some(Duration::from_secs(10)))
                .map_err(|error| error.to_string())?;
        }
        Ok(Self { socket, next_id: 1 })
    }

    fn command(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        self.socket
            .send(Message::Text(
                serde_json::to_string(&json!({ "id": id, "method": method, "params": params }))
                    .map_err(|error| error.to_string())?
                    .into(),
            ))
            .map_err(|error| error.to_string())?;
        loop {
            let value = self.read_value()?;
            if value.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(error) = value.get("error") {
                return Err(cdp_error(error));
            }
            return Ok(value.get("result").cloned().unwrap_or(Value::Null));
        }
    }

    fn evaluate(&mut self, expression: &str) -> Result<Value, String> {
        let result = self.command(
            "Runtime.evaluate",
            json!({
                "expression": expression,
                "returnByValue": true,
                "awaitPromise": false
            }),
        )?;
        ensure_no_exception(&result)?;
        Ok(result
            .pointer("/result/value")
            .cloned()
            .unwrap_or(Value::Null))
    }

    fn read_value(&mut self) -> Result<Value, String> {
        loop {
            match self.socket.read().map_err(|error| error.to_string())? {
                Message::Text(text) => {
                    return serde_json::from_str(&text).map_err(|error| error.to_string())
                }
                Message::Close(_) => return Err("Electron 主进程 Inspector 已关闭。".to_string()),
                _ => {}
            }
        }
    }
}

impl Drop for InspectorSession {
    fn drop(&mut self) {
        let _ = self.socket.close(None);
    }
}

fn cdp_error(error: &Value) -> String {
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("Inspector 命令失败");
    let code = error.get("code").and_then(Value::as_i64).unwrap_or(0);
    format!("{message} ({code})")
}

fn ensure_no_exception(result: &Value) -> Result<(), String> {
    let Some(details) = result.get("exceptionDetails") else {
        return Ok(());
    };
    let description = details
        .pointer("/exception/description")
        .and_then(Value::as_str)
        .or_else(|| details.get("text").and_then(Value::as_str))
        .unwrap_or("未知异常");
    Err(format!(
        "Electron 主进程补丁执行失败：{}",
        description.chars().take(500).collect::<String>()
    ))
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 200
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "._-".contains(character))
}

fn validate_inspector_websocket(value: &str, port: u16) -> Result<String, String> {
    let url = Url::parse(value).map_err(|_| "Electron Inspector 地址无效。".to_string())?;
    let hostname = url.host_str().unwrap_or_default();
    let id = url.path().trim_start_matches('/');
    if url.scheme() != "ws"
        || !matches!(hostname, "127.0.0.1" | "localhost" | "::1")
        || url.port() != Some(port)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !valid_id(id)
    {
        return Err("Electron Inspector 地址未通过本机回环校验。".to_string());
    }
    Ok(url.to_string())
}

fn fetch_json<T: for<'de> Deserialize<'de>>(port: u16, resource: &str) -> Result<T, String> {
    let response = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(2))
        .no_proxy()
        .build()
        .map_err(|error| error.to_string())?
        .get(format!("http://127.0.0.1:{port}{resource}"))
        .header("Cache-Control", "no-store")
        .send()
        .map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!(
            "Inspector 返回 HTTP {}",
            response.status().as_u16()
        ));
    }
    let bytes = response.bytes().map_err(|error| error.to_string())?;
    if bytes.len() > 1024 * 1024 {
        return Err("Inspector 响应超过大小上限。".to_string());
    }
    serde_json::from_slice(&bytes).map_err(|error| error.to_string())
}

fn wait_for_inspector(port: u16) -> Result<InspectorTarget, String> {
    let deadline = Instant::now() + INSPECTOR_WAIT;
    loop {
        let attempt_error = match fetch_json::<Vec<InspectorTarget>>(port, "/json/list") {
            Ok(targets) => {
                if let Some(target) = targets.into_iter().find(|target| {
                    target.target_type == "node"
                        && validate_inspector_websocket(&target.web_socket_debugger_url, port)
                            .is_ok()
                }) {
                    return Ok(target);
                }
                "未找到 Electron Node Inspector target".to_string()
            }
            Err(error) => error,
        };
        if Instant::now() >= deadline {
            return Err(format!(
                "Grok 主进程 Inspector 未能启动（可能被 Electron fuse 禁用）：{attempt_error}"
            ));
        }
        thread::sleep(Duration::from_millis(120));
    }
}

fn select_inspector_port(renderer_port: u16) -> Result<u16, String> {
    for port in PREFERRED_INSPECTOR_PORT..=PREFERRED_INSPECTOR_PORT.saturating_add(100) {
        if port != renderer_port && TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return Ok(port);
        }
    }
    Err("无法为 Grok 分配主进程 Inspector 端口。".to_string())
}

fn schedule_inspector_close(session: &mut InspectorSession) -> Result<(), String> {
    session.evaluate(
        r#"(() => {
  const closeInspector = () => {
    try { process.mainModule.require("inspector").close(); } catch {}
  };
  setTimeout(closeInspector, 150);
  return true;
})()"#,
    )?;
    Ok(())
}

fn wait_for_inspector_close(port: u16) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(4);
    while Instant::now() < deadline {
        if TcpStream::connect_timeout(
            &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
            Duration::from_millis(100),
        )
        .is_err()
        {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }
    Err("Electron 主进程 Inspector 未能按时关闭。".to_string())
}

pub fn launch_with_transparent_wco(
    executable: &Path,
    renderer_port: u16,
) -> Result<String, String> {
    if !executable.is_file() {
        return Err("Grok 可执行文件不存在。".to_string());
    }
    let inspector_port = select_inspector_port(renderer_port)?;
    let mut child = Command::new(executable)
        .args([
            "--remote-debugging-address=127.0.0.1".to_string(),
            format!("--remote-debugging-port={renderer_port}"),
            format!("--inspect=127.0.0.1:{inspector_port}"),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|error| format!("启动 Grok 失败：{error}"))?;

    let result = (|| {
        let target = wait_for_inspector(inspector_port)?;
        let mut session = InspectorSession::open(&target, inspector_port)?;
        session.command("Runtime.enable", json!({}))?;
        let identity = session.evaluate("({ pid: process.pid, type: process.type })")?;
        if identity["pid"].as_u64() != Some(u64::from(child.id()))
            || identity["type"].as_str() != Some("browser")
        {
            return Err("Grok 主进程 Inspector 身份与本次启动的进程不一致。".to_string());
        }
        let patch = session.evaluate(WCO_PATCH)?;
        if patch.get("installed").and_then(Value::as_bool) != Some(true) {
            return Err("Electron 原生标题栏透明补丁未成功安装。".to_string());
        }

        let deadline = Instant::now() + RENDERER_WAIT;
        let browser_id = loop {
            if let Ok(browser_id) = read_browser_identity(renderer_port) {
                if native_titlebar_bridge_ready(renderer_port, &browser_id).unwrap_or(false) {
                    break browser_id;
                }
            }
            if Instant::now() >= deadline {
                return Err("Grok 已启动，但透明原生按钮标题栏未生效。".to_string());
            }
            thread::sleep(Duration::from_millis(250));
        };

        schedule_inspector_close(&mut session)?;
        drop(session);
        wait_for_inspector_close(inspector_port)?;
        Ok(browser_id)
    })();

    if result.is_err() {
        // Do not leave a privileged Inspector listener behind after a failed launch.
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_node_inspector_urls() {
        assert!(validate_inspector_websocket(
            "ws://127.0.0.1:9238/04fc6552-0589-462d-a5b3-7696544e53e6",
            9238
        )
        .is_ok());
        for url in [
            "ws://192.168.1.2:9238/04fc6552-0589-462d-a5b3-7696544e53e6",
            "ws://127.0.0.1:9239/04fc6552-0589-462d-a5b3-7696544e53e6",
            "wss://127.0.0.1:9238/04fc6552-0589-462d-a5b3-7696544e53e6",
            "ws://user@127.0.0.1:9238/04fc6552-0589-462d-a5b3-7696544e53e6",
        ] {
            assert!(validate_inspector_websocket(url, 9238).is_err());
        }
    }
}
