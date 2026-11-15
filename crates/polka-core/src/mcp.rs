//! Протокол MCP (Model Context Protocol) для коннектора «Полки» к приложению Claude:
//! JSON-RPC 2.0 построчно через stdio.
//!
//! Здесь только разбор и ответы: `initialize`, `ping`, `tools/list`, `tools/call`, а ещё
//! доказательство ключа связи и файл расширения `.mcpb`. Сам вызов инструмента — снаружи:
//! посредник `polka --mcp` передаёт его запущенной «Полке».

use crate::tools;
use serde_json::{json, Value};
use std::io::Write;

/// Версии протокола, которые понимает «Полка», — новая первой.
pub const VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// Подсказка для Claude: что это за данные и как с ними обращаться.
pub const INSTRUCTIONS: &str = "«Полка» — личный список прочитанного и просмотренного: книги, фильмы и сериалы \
со статусами want («Хочу»), doing («В процессе»), done («Готово»), dropped («Брошено»), оценками от 1 до 10, датами, \
метками и заметками; у сериалов — отмеченные серии, у книг из читалки — цитаты. Даты — ГГГГ-ММ-ДД. \
Начинайте со сводки (summary); записи ищите через shelf, подробности одной — title. \
Перед записью уточняйте у человека то, чего не знаете: книга это, фильм или сериал, статус, оценка. \
Прежде чем добавить, проверьте через shelf, нет ли записи на полке, — тогда меняйте её через update_title. \
Каждую запись человек видит в «Полке» и может отменить кнопкой «Вернуть». \
Если «Полка» заблокирована или запись выключена, скажите человеку, что сделать в «Полке».";

fn ok(id: &Value, result: Value) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string()
}

fn fail(id: &Value, code: i64, message: &str) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }).to_string()
}

/// Доказательство ключа связи посредника (`polka --mcp`) и запущенной «Полки»: MAC от одноразового
/// числа другой стороны. Сам ключ по сети не ходит, и программа, занявшая порт, ничего не узнает.
pub fn link_proof(token: &str, role: &str, nonce: &str) -> String {
    use blake2::digest::consts::U32;
    use blake2::digest::Mac;
    // Ключ BLAKE2b — не длиннее 64 байт (ключ связи — 64 шестнадцатеричных знака).
    if token.is_empty() || token.len() > 64 {
        return String::new();
    }
    let Ok(mut mac) = blake2::Blake2bMac::<U32>::new_with_salt_and_personal(token.as_bytes(), &[], b"polka-link") else { return String::new() };
    mac.update(role.as_bytes());
    mac.update(b"|");
    mac.update(nonce.as_bytes());
    mac.finalize().into_bytes().iter().map(|b| format!("{b:02x}")).collect()
}

/// Описание инструментов для `tools/list`.
pub fn tool_list() -> Value {
    let tools: Vec<Value> = tools::list()
        .into_iter()
        .map(|t| {
            json!({
                "name": t.name,
                "title": t.title,
                "description": t.description,
                "inputSchema": t.input_schema,
                "annotations": {
                    "title": t.title,
                    "readOnlyHint": !t.write,
                    "destructiveHint": t.name == "delete_title",
                    "idempotentHint": !t.write || matches!(t.name, "update_title" | "mark_episodes" | "set_goal"),
                    "openWorldHint": false
                }
            })
        })
        .collect();
    json!({ "tools": tools })
}

/// Ответ на одну строку запроса; `None` — для уведомлений, на них не отвечают.
/// `call(имя, аргументы)` выполняет инструмент: `Ok` — данные, `Err` — понятная человеку ошибка.
pub fn handle(line: &str, version: &str, call: &mut dyn FnMut(&str, &Value) -> Result<Value, String>) -> Option<String> {
    let msg: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return Some(fail(&Value::Null, -32700, "Parse error")),
    };
    if !msg.is_object() {
        return Some(fail(&Value::Null, -32600, "Invalid Request"));
    }
    let method = msg.get("method").and_then(|m| m.as_str());
    let id = msg.get("id").cloned();
    let params = msg.get("params").cloned().unwrap_or_else(|| json!({}));
    // Ответ клиента на наш запрос (мы их не шлём) или уведомление — молча.
    let (Some(method), Some(id)) = (method, id) else { return None };
    Some(match method {
        "initialize" => {
            let asked = params.get("protocolVersion").and_then(|v| v.as_str()).unwrap_or("");
            let version_used = if VERSIONS.contains(&asked) { asked } else { VERSIONS[0] };
            ok(
                &id,
                json!({
                    "protocolVersion": version_used,
                    "capabilities": { "tools": { "listChanged": false } },
                    "serverInfo": { "name": "polka", "title": "Полка", "version": version },
                    "instructions": INSTRUCTIONS
                }),
            )
        }
        "ping" => ok(&id, json!({})),
        "tools/list" => ok(&id, tool_list()),
        "tools/call" => {
            let name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
            if tools::find(name).is_none() {
                return Some(fail(&id, -32602, &format!("Unknown tool: {name}")));
            }
            let args = match params.get("arguments") {
                None | Some(Value::Null) => json!({}),
                Some(a) if a.is_object() => a.clone(),
                Some(_) => return Some(fail(&id, -32602, "arguments must be an object")),
            };
            match call(name, &args) {
                Ok(v) => ok(&id, json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(&v).unwrap_or_default() }], "isError": false })),
                Err(e) => ok(&id, json!({ "content": [{ "type": "text", "text": e }], "isError": true })),
            }
        }
        "resources/list" => ok(&id, json!({ "resources": [] })),
        "resources/templates/list" => ok(&id, json!({ "resourceTemplates": [] })),
        "prompts/list" => ok(&id, json!({ "prompts": [] })),
        _ => fail(&id, -32601, &format!("Method not found: {method}")),
    })
}

/// Описание расширения для приложения Claude (`manifest.json` внутри `.mcpb`).
/// `command` — полный путь к `polka.exe`, `dir` — папка данных «Полки», `platform` — win32, darwin или linux.
pub fn manifest(version: &str, command: &str, dir: &str, platform: &str) -> Value {
    // Путь может быть виндовым и там, где собирают проверки, — режем по обоим разделителям.
    let entry = command.rsplit(['/', '\\']).next().filter(|n| !n.is_empty()).unwrap_or("polka");
    json!({
        "manifest_version": "0.3",
        "name": "polka",
        "display_name": "Полка",
        "version": version,
        "description": "Прочитанные книги, просмотренные фильмы и сериалы из «Полки»: полка, «Хочу», итоги года, цитаты. Запись — если она разрешена в «Полке».",
        "long_description": "Расширение запускает «Полку» на этом компьютере в режиме коннектора. Данные не уходят в интернет, кроме того, что Claude прочитает по вашей просьбе. Включается в «Полке»: Настройки → Claude.",
        "author": { "name": "Полка" },
        "icon": "icon.png",
        "server": {
            "type": "binary",
            "entry_point": entry,
            "mcp_config": { "command": command, "args": ["--mcp"], "env": { "POLKA_DIR": dir } }
        },
        "tools": tools::list().iter().map(|t| json!({ "name": t.name, "description": t.title })).collect::<Vec<_>>(),
        "compatibility": { "platforms": [platform] },
        "keywords": ["books", "films", "series", "reading"],
        "license": "Proprietary"
    })
}

/// Файл расширения `.mcpb`: архив ZIP с `manifest.json` и значком.
pub fn bundle(manifest: &Value, icon_png: &[u8]) -> Result<Vec<u8>, String> {
    let err = |e: zip::result::ZipError| format!("расширение не собралось: {e}");
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    zw.start_file("manifest.json", opts).map_err(err)?;
    zw.write_all(&serde_json::to_vec_pretty(manifest).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    // PNG уже сжат — кладём как есть.
    zw.start_file("icon.png", opts.compression_method(zip::CompressionMethod::Stored)).map_err(err)?;
    zw.write_all(icon_png).map_err(|e| e.to_string())?;
    Ok(zw.finish().map_err(err)?.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_proof_depends_on_key_role_and_nonce() {
        let token = "ab".repeat(32);
        let p = link_proof(&token, "polka", "01".repeat(32).as_str());
        assert_eq!(p.len(), 64);
        assert_eq!(p, link_proof(&token, "polka", "01".repeat(32).as_str()), "одинаково для одних и тех же данных");
        assert_ne!(p, link_proof(&token, "bridge", "01".repeat(32).as_str()), "у сторон разные доказательства");
        assert_ne!(p, link_proof(&token, "polka", "02".repeat(32).as_str()));
        assert_ne!(p, link_proof(&"cd".repeat(32), "polka", "01".repeat(32).as_str()));
        assert!(!p.contains(&token));
        assert_eq!(link_proof(&"x".repeat(65), "polka", "01"), "", "ключ длиннее 64 байт не подходит");
        assert_eq!(link_proof("", "polka", "01"), "");
    }

    fn run(line: &str) -> Value {
        let out = handle(line, "0.10.0", &mut |name, _args| if name == "summary" { Ok(json!({ "books_done": 24 })) } else { Err("«Полка» заблокирована".into()) });
        serde_json::from_str(&out.expect("ответ")).unwrap()
    }

    #[test]
    fn handshake_lists_tools_and_calls_them() {
        let r = run(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"claude-ai","version":"1"}}}"#);
        assert_eq!(r["id"], 1);
        assert_eq!(r["result"]["protocolVersion"], "2025-06-18", "версию клиента повторяем, если знаем её");
        assert_eq!(r["result"]["serverInfo"]["name"], "polka");
        let r = run(r#"{"jsonrpc":"2.0","id":2,"method":"initialize","params":{"protocolVersion":"2099-01-01"}}"#);
        assert_eq!(r["result"]["protocolVersion"], VERSIONS[0], "незнакомую — заменяем своей новой");
        assert_eq!(handle(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#, "x", &mut |_, _| Ok(json!(null))), None);
        let r = run(r#"{"jsonrpc":"2.0","id":"a","method":"tools/list"}"#);
        let list = r["result"]["tools"].as_array().unwrap();
        assert_eq!(list.len(), tools::list().len());
        let add = list.iter().find(|t| t["name"] == "add_title").unwrap();
        assert_eq!(add["annotations"]["readOnlyHint"], false);
        assert_eq!(add["inputSchema"]["required"][0], "kind");
        let del = list.iter().find(|t| t["name"] == "delete_title").unwrap();
        assert_eq!(del["annotations"]["destructiveHint"], true);
        let shelf = list.iter().find(|t| t["name"] == "shelf").unwrap();
        assert_eq!(shelf["annotations"]["readOnlyHint"], true);
        let r = run(r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"summary","arguments":{}}}"#);
        assert_eq!(r["result"]["isError"], false);
        assert!(r["result"]["content"][0]["text"].as_str().unwrap().contains("24"));
        let r = run(r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"shelf"}}"#);
        assert_eq!(r["result"]["isError"], true, "ошибка инструмента — в ответе, а не сбоем протокола");
        assert_eq!(r["result"]["content"][0]["text"], "«Полка» заблокирована");
        let r = run(r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"rm_rf"}}"#);
        assert_eq!(r["error"]["code"], -32602);
        let r = run(r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"shelf","arguments":[1]}}"#);
        assert_eq!(r["error"]["code"], -32602);
        assert_eq!(run(r#"{"jsonrpc":"2.0","id":7,"method":"ping"}"#)["result"], json!({}));
        assert_eq!(run(r#"{"jsonrpc":"2.0","id":8,"method":"sampling/createMessage"}"#)["error"]["code"], -32601);
        assert_eq!(run("не json")["error"]["code"], -32700);
        assert_eq!(run("[1,2]")["error"]["code"], -32600);
    }

    #[test]
    fn bundle_is_a_zip_with_a_valid_manifest() {
        let m = manifest("0.10.0", r"C:\Users\Пётр\AppData\Local\Polka\polka.exe", r"C:\Users\Пётр\AppData\Roaming\io.github.primusfixer.polka", "win32");
        assert_eq!(m["server"]["entry_point"], "polka.exe");
        assert_eq!(m["server"]["mcp_config"]["args"][0], "--mcp");
        assert_eq!(m["tools"].as_array().unwrap().len(), tools::list().len());
        let data = bundle(&m, b"\x89PNG\r\n\x1a\nicon").unwrap();
        assert_eq!(&data[..4], b"PK\x03\x04");
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(data)).unwrap();
        let mut text = String::new();
        std::io::Read::read_to_string(&mut z.by_name("manifest.json").unwrap(), &mut text).unwrap();
        let back: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(back["server"]["mcp_config"]["env"]["POLKA_DIR"], r"C:\Users\Пётр\AppData\Roaming\io.github.primusfixer.polka");
        assert_eq!(z.by_name("icon.png").unwrap().size(), 12);
    }
}
