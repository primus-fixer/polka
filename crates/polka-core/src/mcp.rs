//! Коннектор «Полки» к приложению Claude. Протокол MCP, доказательство ключа связи и файл расширения
//! `.mcpb` — общие из kit (`kit_core::mcp`); здесь то, чем «Полка» отличается: имя, подсказка для Claude,
//! строка ключа связи, описание расширения и её инструменты ([`crate::tools`]).

use crate::tools;
use kit_core::mcp::{self as kit, Extension, Server, Tool};
use serde_json::Value;

pub use kit::{bundle, VERSIONS};

/// Подсказка для Claude: что это за данные и как с ними обращаться.
pub const INSTRUCTIONS: &str = "«Полка» — личный список прочитанного и просмотренного: книги, фильмы и сериалы \
со статусами want («Хочу»), doing («В процессе»), done («Готово»), dropped («Брошено»), оценками от 1 до 10, датами, \
метками и заметками; у сериалов — отмеченные серии, у книг из читалки — цитаты. Даты — ГГГГ-ММ-ДД. \
Начинайте со сводки (summary); записи ищите через shelf, подробности одной — title. \
Перед записью уточняйте у человека то, чего не знаете: книга это, фильм или сериал, статус, оценка. \
Прежде чем добавить, проверьте через shelf, нет ли записи на полке, — тогда меняйте её через update_title. \
Каждую запись человек видит в «Полке» и может отменить кнопкой «Вернуть». \
Если «Полка» заблокирована или запись выключена, скажите человеку, что сделать в «Полке».";

/// Коннектор «Полки». Строку ключа связи менять нельзя: посредник прежней версии и запущенная «Полка»
/// перестанут узнавать друг друга.
pub const SERVER: Server = Server { name: "polka", title: "Полка", instructions: INSTRUCTIONS, link_personal: b"polka-link" };

/// Расширение «Полки» для приложения Claude.
pub const EXTENSION: Extension = Extension {
    display_name: "Полка",
    description: "Прочитанные книги, просмотренные фильмы и сериалы из «Полки»: полка, «Хочу», итоги года, цитаты. Запись — если она разрешена в «Полке».",
    long_description: "Расширение запускает «Полку» на этом компьютере в режиме коннектора. Данные не уходят в интернет, кроме того, что Claude прочитает по вашей просьбе. Включается в «Полке»: Настройки → Claude.",
    author: "Полка",
    keywords: &["books", "films", "series", "reading"],
    dir_env: "POLKA_DIR",
};

/// Инструменты «Полки» для протокола: удаляет только `delete_title`; повторять можно чтение,
/// правку записи, отметку серий и цель.
pub fn tools() -> Vec<Tool> {
    tools::list()
        .into_iter()
        .map(|t| Tool {
            destructive: t.name == "delete_title",
            idempotent: matches!(t.name, "update_title" | "mark_episodes" | "set_goal"),
            name: t.name,
            title: t.title,
            description: t.description,
            input_schema: t.input_schema,
            write: t.write,
        })
        .collect()
}

/// Доказательство ключа связи посредника (`polka --mcp`) и запущенной «Полки».
pub fn link_proof(token: &str, role: &str, nonce: &str) -> String {
    kit::link_proof(&SERVER, token, role, nonce)
}

/// Описание инструментов для `tools/list`.
pub fn tool_list() -> Value {
    kit::tool_list(&tools())
}

/// Ответ на одну строку запроса; `None` — для уведомлений, на них не отвечают.
/// `call(имя, аргументы)` выполняет инструмент: `Ok` — данные, `Err` — понятная человеку ошибка.
pub fn handle(line: &str, version: &str, call: &mut dyn FnMut(&str, &Value) -> Result<Value, String>) -> Option<String> {
    kit::handle(&SERVER, &tools(), line, version, call)
}

/// Описание расширения для приложения Claude (`manifest.json` внутри `.mcpb`).
/// `command` — полный путь к `polka.exe`, `dir` — папка данных «Полки», `platform` — win32, darwin или linux.
pub fn manifest(version: &str, command: &str, dir: &str, platform: &str) -> Value {
    kit::manifest(&SERVER, &EXTENSION, &tools(), version, command, dir, platform)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
