//! Built-in MCP server (Model Context Protocol), so an AI assistant can work with the app:
//! read and change projects, watch the group monitor and send telegrams.
//!
//! The server listens on this computer only (http://127.0.0.1:<port>/mcp, "Streamable HTTP"
//! with plain JSON answers). Assistants that only start programs (stdio) use
//! `buscraft --mcp-stdio`, which passes each message on to the running app.
//! Tool calls are carried out by the app's own window loop, so changes show up at once and
//! can be undone like any other edit.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const DEFAULT_PORT: u16 = 47811;
const PROTOCOL: &str = "2025-06-18";
const SUPPORTED: [&str; 4] = ["2025-06-18", "2025-03-26", "2024-11-05", "2025-11-25"];

/// A tool call waiting for the window loop.
pub struct Call {
    pub name: String,
    pub args: Value,
    pub reply: Sender<Result<Value, String>>,
}

pub struct McpServer {
    rx: Option<Receiver<Call>>,
    stop: Arc<AtomicBool>,
    ctx: Arc<Mutex<Option<egui::Context>>>,
    pub port: u16,
    pub running: bool,
    pub status: String,
    pub calls: u64,
    pub last_call: String,
}

impl Default for McpServer {
    fn default() -> Self {
        McpServer { rx: None, stop: Arc::new(AtomicBool::new(false)), ctx: Default::default(), port: DEFAULT_PORT, running: false, status: "Off".into(), calls: 0, last_call: String::new() }
    }
}

impl McpServer {
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/mcp", self.port)
    }

    /// Start listening (stops a running server first).
    pub fn start(&mut self, port: u16) {
        self.stop();
        self.port = port;
        let listener = match TcpListener::bind(("127.0.0.1", port)) {
            Ok(l) => l,
            Err(e) => {
                self.status = format!("Could not use port {port}: {e}");
                return;
            }
        };
        let _ = listener.set_nonblocking(true);
        let (tx, rx) = channel::<Call>();
        let stop = Arc::new(AtomicBool::new(false));
        let ctx = self.ctx.clone();
        {
            let stop = stop.clone();
            std::thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let tx = tx.clone();
                            let ctx = ctx.clone();
                            std::thread::spawn(move || handle_connection(stream, &tx, &ctx));
                        }
                        Err(_) => std::thread::sleep(Duration::from_millis(40)),
                    }
                }
            });
        }
        self.rx = Some(rx);
        self.stop = stop;
        self.running = true;
        self.status = format!("Listening on {}", self.url());
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.rx = None;
        if self.running {
            self.status = "Off".into();
        }
        self.running = false;
    }

    /// Give the server the window context, so a call can wake the window loop.
    pub fn set_ctx(&self, ctx: &egui::Context) {
        if let Ok(mut c) = self.ctx.lock() {
            if c.is_none() {
                *c = Some(ctx.clone());
            }
        }
    }

    pub fn take_calls(&mut self) -> Vec<Call> {
        let Some(rx) = &self.rx else { return vec![] };
        let calls: Vec<Call> = rx.try_iter().collect();
        self.calls += calls.len() as u64;
        if let Some(c) = calls.last() {
            self.last_call = c.name.clone();
        }
        calls
    }
}

// ------------------------------------------------------------------ HTTP

struct Request {
    method: String,
    path: String,
    origin: Option<String>,
    body: Vec<u8>,
}

fn read_request(stream: &mut TcpStream) -> Option<Request> {
    let mut r = BufReader::new(stream.try_clone().ok()?);
    let mut line = String::new();
    r.read_line(&mut line).ok()?;
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();
    let mut len = 0usize;
    let mut origin = None;
    loop {
        let mut h = String::new();
        if r.read_line(&mut h).ok()? == 0 {
            break;
        }
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            match k.trim().to_ascii_lowercase().as_str() {
                "content-length" => len = v.trim().parse().unwrap_or(0),
                "origin" => origin = Some(v.trim().to_string()),
                _ => {}
            }
        }
    }
    if len > 8 * 1024 * 1024 {
        return None;
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body).ok()?;
    Some(Request { method, path, origin, body })
}

fn respond(stream: &mut TcpStream, status: &str, body: &str, extra: &str) {
    let ct = if body.is_empty() { "" } else { "Content-Type: application/json\r\n" };
    let head = format!("HTTP/1.1 {status}\r\n{ct}Content-Length: {}\r\n{extra}Connection: close\r\n\r\n", body.len());
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body.as_bytes());
    let _ = stream.flush();
    let _ = stream.shutdown(Shutdown::Write);
}

fn local_origin(o: &str) -> bool {
    let host = o.split("://").nth(1).unwrap_or(o);
    let host = host.split(['/', ':']).next().unwrap_or("");
    matches!(host, "localhost" | "127.0.0.1" | "[::1]" | "") || o == "null"
}

fn handle_connection(mut stream: TcpStream, tx: &Sender<Call>, ctx: &Arc<Mutex<Option<egui::Context>>>) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let Some(req) = read_request(&mut stream) else { return };
    let path = req.path.split('?').next().unwrap_or("");
    if path != "/mcp" && path != "/" {
        respond(&mut stream, "404 Not Found", "", "");
        return;
    }
    // Refuse web pages from other sites (DNS rebinding protection).
    if let Some(o) = &req.origin {
        if !local_origin(o) {
            respond(&mut stream, "403 Forbidden", "", "");
            return;
        }
    }
    match req.method.as_str() {
        "POST" => {}
        "OPTIONS" => {
            respond(&mut stream, "204 No Content", "", "Allow: POST\r\n");
            return;
        }
        _ => {
            respond(&mut stream, "405 Method Not Allowed", "", "Allow: POST\r\n");
            return;
        }
    }
    let msg: Value = match serde_json::from_slice(&req.body) {
        Ok(v) => v,
        Err(e) => {
            let err = json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": format!("Parse error: {e}")}});
            respond(&mut stream, "400 Bad Request", &err.to_string(), "");
            return;
        }
    };
    let answer = if let Some(batch) = msg.as_array() {
        let out: Vec<Value> = batch.iter().filter_map(|m| handle_message(m, tx, ctx)).collect();
        if out.is_empty() { None } else { Some(Value::Array(out)) }
    } else {
        handle_message(&msg, tx, ctx)
    };
    match answer {
        Some(a) => respond(&mut stream, "200 OK", &a.to_string(), ""),
        None => respond(&mut stream, "202 Accepted", "", ""),
    }
}

fn rpc_error(id: &Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

/// Answer one JSON-RPC message (None for notifications).
fn handle_message(msg: &Value, tx: &Sender<Call>, ctx: &Arc<Mutex<Option<egui::Context>>>) -> Option<Value> {
    let id = msg.get("id").cloned()?;
    let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let params = msg.get("params").cloned().unwrap_or(json!({}));
    let result = match method {
        "initialize" => {
            let asked = params.get("protocolVersion").and_then(|v| v.as_str()).unwrap_or(PROTOCOL);
            let version = if SUPPORTED.contains(&asked) { asked } else { PROTOCOL };
            json!({
                "protocolVersion": version,
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": "buscraft", "title": "BusCraft", "version": env!("CARGO_PKG_VERSION")},
                "instructions": INSTRUCTIONS,
            })
        }
        "ping" => json!({}),
        "tools/list" => json!({"tools": tool_list()}),
        "tools/call" => {
            let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
            if !tool_list().iter().any(|t| t["name"] == name.as_str()) {
                return Some(rpc_error(&id, -32602, &format!("Unknown tool: {name}")));
            }
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            let out = run_in_app(tx, ctx, name, args);
            match out {
                Ok(v) => json!({"content": [{"type": "text", "text": serde_json::to_string_pretty(&v).unwrap_or_default()}], "structuredContent": wrap(v), "isError": false}),
                Err(e) => json!({"content": [{"type": "text", "text": e}], "isError": true}),
            }
        }
        "resources/list" => json!({"resources": []}),
        "prompts/list" => json!({"prompts": []}),
        _ => return Some(rpc_error(&id, -32601, &format!("Method not found: {method}"))),
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

/// Structured content must be an object.
fn wrap(v: Value) -> Value {
    if v.is_object() { v } else { json!({"result": v}) }
}

fn run_in_app(tx: &Sender<Call>, ctx: &Arc<Mutex<Option<egui::Context>>>, name: String, args: Value) -> Result<Value, String> {
    let (rtx, rrx) = channel();
    tx.send(Call { name, args, reply: rtx }).map_err(|_| "The app is closing".to_string())?;
    if let Ok(c) = ctx.lock() {
        if let Some(c) = c.as_ref() {
            c.request_repaint();
        }
    }
    rrx.recv_timeout(Duration::from_secs(20)).map_err(|_| "The app did not answer in time (is a dialog open?)".to_string())?
}

// ------------------------------------------------------------------ stdio bridge

/// `buscraft --mcp-stdio`: read JSON-RPC lines on stdin, pass them to the running app, write
/// the answers to stdout. For assistants that start MCP servers as programs.
pub fn stdio_bridge(port: u16) {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let id = serde_json::from_str::<Value>(&line).ok().and_then(|v| v.get("id").cloned());
        match post(port, &line) {
            Ok(body) if !body.trim().is_empty() => {
                let _ = writeln!(out, "{}", body.trim());
            }
            Ok(_) => {}
            Err(e) => {
                if let Some(id) = id {
                    let msg = format!("BusCraft is not reachable on port {port} ({e}). Start BusCraft and switch on the MCP server in Settings.");
                    let _ = writeln!(out, "{}", rpc_error(&id, -32000, &msg));
                }
            }
        }
        let _ = out.flush();
    }
}

#[cfg(test)]
pub fn test_post(port: u16, body: &str) -> Result<String, String> {
    post(port, body)
}

fn post(port: u16, body: &str) -> Result<String, String> {
    let mut s = TcpStream::connect(("127.0.0.1", port)).map_err(|e| e.to_string())?;
    let _ = s.set_read_timeout(Some(Duration::from_secs(30)));
    let req = format!("POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    s.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    let mut resp = Vec::new();
    s.read_to_end(&mut resp).map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&resp).to_string();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let status = head.split_whitespace().nth(1).unwrap_or("");
    if status.starts_with('2') {
        Ok(body.to_string())
    } else {
        Err(format!("HTTP {status}"))
    }
}

// ------------------------------------------------------------------ tool descriptions

const INSTRUCTIONS: &str = "BusCraft is a KNX building automation planning and commissioning tool. \
Projects contain devices (individual addresses like 1.1.5 on area.line), group addresses (like 1/2/3), \
device parameters and group objects that are linked to group addresses, and a building structure. \
Tools work on the project open in the app unless 'project' is given. Changes to the open project can be undone in the app. \
Changed devices must be downloaded (programmed) from the app afterwards. \
read_telegrams shows the group monitor; send_group_value writes to the bus through the app's selected interface.";

fn s(desc: &str) -> Value {
    json!({"type": "string", "description": desc})
}
fn n(desc: &str) -> Value {
    json!({"type": "integer", "description": desc})
}
fn b(desc: &str) -> Value {
    json!({"type": "boolean", "description": desc})
}

fn tool(name: &str, title: &str, desc: &str, props: Value, required: &[&str], read_only: bool) -> Value {
    let mut p = props;
    if let Some(o) = p.as_object_mut() {
        o.insert("project".into(), s("Project name or id (default: the project open in the app)"));
    }
    json!({
        "name": name,
        "title": title,
        "description": desc,
        "inputSchema": {"type": "object", "properties": p, "required": required},
        "annotations": {"readOnlyHint": read_only, "destructiveHint": name.starts_with("delete"), "openWorldHint": name.starts_with("send")},
    })
}

pub fn tool_list() -> Vec<Value> {
    let dev = || s("Device: individual address (1.1.5), name or id");
    let ga = || s("Group address, e.g. 1/2/3 (also 1/515 or 2563)");
    vec![
        json!({"name": "list_projects", "title": "List projects", "description": "All projects with id, name, status, counts and which one is open.",
               "inputSchema": {"type": "object", "properties": {}}, "annotations": {"readOnlyHint": true}}),
        json!({"name": "open_project", "title": "Open project", "description": "Open a project in the app (by name or id).",
               "inputSchema": {"type": "object", "properties": {"project": s("Project name or id")}, "required": ["project"]}, "annotations": {"readOnlyHint": false}}),
        tool("get_project", "Project overview", "Project details: topology (areas, lines with device counts), building structure, group address ranges and counts.", json!({}), &[], true),
        tool("list_devices", "List devices", "Devices with address, name, product, room and commissioning status.", json!({"filter": s("Only devices whose address, name, product or room contains this text")}), &[], true),
        tool("get_device", "Device details", "One device with its parameters and group objects (with linked group addresses) and commissioning state.", json!({"device": dev()}), &["device"], true),
        tool("list_group_addresses", "List group addresses", "Group addresses with name, data type, linked objects and last value seen on the bus.", json!({"filter": s("Only addresses whose address, name or data type contains this text")}), &[], true),
        tool("list_catalog", "Search product catalog", "Products that can be added to the project.", json!({"filter": s("Text in manufacturer, name, order number or category"), "limit": n("Most results (default 50)")}), &[], true),
        tool("create_group_address", "Create group address", "Create a group address. Give 'address' (1/2/3), or 'main' and 'middle' to take the next free one. Missing main/middle groups are created.",
             json!({"address": ga(), "main": n("Main group (0-31)"), "middle": n("Middle group (0-7)"), "name": s("Name"), "dpt": s("Data type, e.g. '1.001 switch', '5.001', '9.001', '10.001', '11.001'"), "central": b("Central function"), "comment": s("Comment")}), &["name"], false),
        tool("update_group_address", "Change group address", "Change name, data type, central flag, comment, or move it to another address.",
             json!({"address": ga(), "name": s("New name"), "dpt": s("New data type"), "central": b("Central function"), "comment": s("Comment"), "new_address": s("Move to this address")}), &["address"], false),
        tool("delete_group_address", "Delete group address", "Delete a group address and remove its links from all group objects.", json!({"address": ga()}), &["address"], false),
        tool("name_group_range", "Name main or middle group", "Create or rename a main group ('1') or middle group ('1/2').", json!({"range": s("'1' for a main group or '1/2' for a middle group"), "name": s("Name")}), &["range", "name"], false),
        tool("link_group_address", "Link group object", "Link a device's group object to a group address. 'sending' makes it the object's first (sending) address.",
             json!({"device": dev(), "object": n("Group object number"), "address": ga(), "sending": b("Make it the sending address")}), &["device", "object", "address"], false),
        tool("unlink_group_address", "Unlink group object", "Remove the link between a group object and a group address.", json!({"device": dev(), "object": n("Group object number"), "address": ga()}), &["device", "object", "address"], false),
        tool("add_device", "Add device", "Add a product from the catalog to a line.",
             json!({"product": s("Product name or order number from list_catalog"), "area": n("Area (default 1)"), "line": n("Line (default 1)"), "number": n("Device number (default: next free)"), "name": s("Device name"), "space": s("Room or distribution board to put it in")}), &["product"], false),
        tool("update_device", "Change device", "Rename a device, change description or comment, or give it another individual address.",
             json!({"device": dev(), "name": s("New name"), "description": s("Description"), "comment": s("Comment"), "new_address": s("New individual address, e.g. 1.1.12")}), &["device"], false),
        tool("delete_device", "Delete device", "Remove a device from the project.", json!({"device": dev()}), &["device"], false),
        tool("set_parameter", "Set parameter", "Set a device parameter (see get_device for names and allowed values).",
             json!({"device": dev(), "parameter": s("Parameter name"), "page": s("Parameter page, if the name is used on more pages"), "value": s("New value")}), &["device", "parameter", "value"], false),
        tool("create_space", "Create building part or room", "Add a building part, floor, room, corridor, stairway or distribution board to the building structure.",
             json!({"name": s("Name"), "kind": s("building, building part, floor, room, corridor, stairway or distribution board"), "parent": s("Name of the parent (default: the first building)")}), &["name", "kind"], false),
        tool("assign_device", "Put device in a room", "Put a device in a room, corridor, stairway or distribution board.", json!({"device": dev(), "space": s("Name of the room or distribution board")}), &["device", "space"], false),
        tool("bus_status", "Bus status", "The selected bus interface, whether it is connected and whether the group monitor is recording.", json!({}), &[], true),
        tool("read_telegrams", "Read group monitor", "Telegrams recorded by the group monitor, oldest first. Use 'after' with the last seq you saw to get only new ones.",
             json!({"after": n("Only telegrams with a higher seq"), "limit": n("Most telegrams (default 100, newest kept)"), "address": s("Only telegrams to this group address")}), &[], true),
        tool("send_group_value", "Write value", "Send a GroupValueWrite to a group address through the app's interface. The value is typed as text and encoded with the address's data type (e.g. 1/0, 50 for 5.001, 21.5 for 9.001, 'Mon 14:30:00' or 'now' for 10.001, '25.11.2026' or 'today' for 11.001).",
             json!({"address": ga(), "value": s("Value")}), &["address", "value"], false),
        tool("read_group_value", "Read value", "Send a GroupValueRead to a group address; answers appear in read_telegrams.", json!({"address": ga()}), &["address"], false),
    ]
}
