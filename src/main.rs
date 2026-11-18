use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::process::{Command, Stdio};
use tiny_http::{Header, Response, Server, StatusCode};

fn cors_header() -> Header {
    Header::from_bytes(&b"Access-Control-Allow-Origin"[..], &b"*"[..]).unwrap()
}

fn url_decode(s: &str) -> String {
    let mut out = String::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'+' {
            out.push(' ');
            i += 1;
        } else if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(hex) = std::str::from_utf8(&bytes[i + 1..i + 3]) {
                if let Ok(val) = u8::from_str_radix(hex, 16) {
                    out.push(val as char);
                    i += 3;
                    continue;
                }
            }
            out.push('%');
            i += 1;
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

fn get_param(url: &str, key: &str) -> Option<String> {
    let (_, query) = url.split_once('?')?;
    for pair in query.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            if k == key {
                return Some(url_decode(v));
            }
        }
    }
    None
}

fn main() {
    let server = Server::http("0.0.0.0:8080").expect("Не удалось запустить сервер на порту 8080");
    println!("Сервер запущен: http://127.0.0.1:8080");

    let mut search_cache: HashMap<String, String> = HashMap::new();

    for request in server.incoming_requests() {
        let url = request.url().to_string();

        // 1. Отдаём index.html
        if url == "/" || url.starts_with("/index.html") {
            match fs::read_to_string(r"C:\dsa\index.html") {
                Ok(html) => {
                    let mut resp = Response::from_string(html);
                    resp.add_header(Header::from_bytes(&b"Content-Type"[..], &b"text/html; charset=utf-8"[..]).unwrap());
                    let _ = request.respond(resp);
                }
                Err(e) => {
                    let _ = request.respond(Response::from_string(format!("Ошибка чтения C:\\dsa\\index.html: {}", e)).with_status_code(500));
                }
            }
        }
        // 2. Поиск треков с кэшированием
        else if url.starts_with("/search") {
            let query = get_param(&url, "q").unwrap_or_default().to_lowercase();

            let body = if let Some(cached) = search_cache.get(&query) {
                println!("[КЭШ ПОИСКА] {}", query);
                cached.clone()
            } else {
                println!("[YT-DLP ПОИСК] {}", query);
                let search_arg = format!("scsearch15:{}", query);
                let output = Command::new("./yt-dlp.exe")
                    .args(["--dump-json", "--flat-playlist", "--no-warnings", &search_arg])
                    .output();

                let mut tracks = Vec::new();
                if let Ok(out) = output {
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    for line in stdout.lines() {
                        if let Ok(v) = serde_json::from_str::<Value>(line) {
                            tracks.push(json!({
                                "title": v["title"].as_str().unwrap_or("Без названия"),
                                "uploader": v["uploader"].as_str().unwrap_or("Неизвестен"),
                                "url": v["url"].as_str().unwrap_or(""),
                                "duration": v["duration"].as_f64().unwrap_or(0.0)
                            }));
                        }
                    }
                }
                let json_str = serde_json::to_string(&tracks).unwrap_or_else(|_| "[]".to_string());
                if !tracks.is_empty() {
                    search_cache.insert(query, json_str.clone());
                }
                json_str
            };

            let mut resp = Response::from_string(body);
            resp.add_header(Header::from_bytes(&b"Content-Type"[..], &b"application/json; charset=utf-8"[..]).unwrap());
            resp.add_header(cors_header());
            let _ = request.respond(resp);
        }
        // 3. Стриминг аудио напрямую через yt-dlp в stdout
        else if url.starts_with("/stream") {
            let track_url = get_param(&url, "url").unwrap_or_default();
            println!("[СТРИМ] Запуск потока: {}", track_url);

            let child = Command::new("./yt-dlp.exe")
                .args([
                    "-f", "bestaudio[ext=mp3]/bestaudio",
                    "--no-warnings",
                    "-o", "-",
                    &track_url,
                ])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn();

            if let Ok(mut process) = child {
                if let Some(stdout) = process.stdout.take() {
                    let headers = vec![
                        cors_header(),
                        Header::from_bytes(&b"Content-Type"[..], &b"audio/mpeg"[..]).unwrap(),
                    ];
                    let resp = Response::new(
                        StatusCode(200),
                        headers,
                        stdout,
                        None,
                        None,
                    );
                    let _ = request.respond(resp);
                    let _ = process.wait();
                    continue;
                }
            }
            let _ = request.respond(Response::from_string("Ошибка загрузки потока").with_status_code(502));
        } else {
            let _ = request.respond(Response::from_string("Not Found").with_status_code(404));
        }
    }
}