//! カタンのオンライン対戦サーバ。
//!
//!   catan-server [ポート] [配るフォルダ]
//!
//! 静的ファイル（web/）とオンライン対戦の口を、同じ場所から出す。
//! 同じ出どころなので CORS の設定が要らない。
//!
//! ⛔ 外部に常設するものではない。身内で遊ぶ間だけ立てる想定。

mod http;
mod rooms;

use rooms::{new_code, new_token, now_ms, Member, Phase, Room};
use std::collections::HashMap;
use std::net::{TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

type Rooms = Arc<Mutex<HashMap<String, Room>>>;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let port: u16 = args.first().and_then(|s| s.parse().ok()).unwrap_or(8788);
    let root = PathBuf::from(args.get(1).cloned().unwrap_or_else(|| "web".into()));

    let rooms: Rooms = Arc::new(Mutex::new(HashMap::new()));

    // CPU を進める係。部屋ごとにスレッドを立てると数が読めないので、1 本で回す
    {
        let rooms = Arc::clone(&rooms);
        thread::spawn(move || loop {
            {
                let mut r = rooms.lock().unwrap();
                for room in r.values_mut() {
                    room.step_bot();
                }
                // 誰も居なくなった部屋は畳む（30 分）
                let now = now_ms();
                r.retain(|_, room| {
                    let alive = room.members.iter().any(|m| !m.feeds.is_empty());
                    if alive {
                        room.last_touch = now;
                    }
                    now - room.last_touch < 30 * 60 * 1000
                });
            }
            thread::sleep(Duration::from_millis(120));
        });
    }

    let listener = match TcpListener::bind(("0.0.0.0", port)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("ポート {port} を開けません: {e}");
            std::process::exit(1);
        }
    };
    println!("カタン サーバ: http://localhost:{port}/  （配信元 {}）", root.display());
    println!("止めるときは Ctrl+C");

    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let rooms = Arc::clone(&rooms);
        let root = root.clone();
        thread::spawn(move || handle(stream, rooms, root));
    }
}

fn handle(mut stream: TcpStream, rooms: Rooms, root: PathBuf) {
    let Some(req) = http::read_request(&stream) else { return };

    // 対戦の口は**どの階層の下でも**受ける。
    // UI は相対パス（`api/...`）で呼ぶので、`/colonist/` から開くと
    // `/colonist/api/create` に飛ぶ。ここで拾わないとオンライン対戦だけが 404 になる
    if let Some(i) = req.path.find("/api/") {
        let rest = &req.path[i + 5..];
        api(&mut stream, rest, &req, &rooms);
        return;
    }
    serve_static(&mut stream, &req.path, &root);
}

// ---------------------------------------------------------------- 静的ファイル

fn serve_static(stream: &mut TcpStream, path: &str, root: &Path) {
    // 「/」で終わる指定はその中の index.html を配る（/colonist/ など）
    let rel: String = if path.ends_with('/') {
        format!("{}index.html", path.trim_start_matches('/'))
    } else {
        path.trim_start_matches('/').to_string()
    };
    let rel = rel.as_str();
    // 上へ辿る指定は受け付けない
    let mut safe = PathBuf::new();
    for c in Path::new(rel).components() {
        match c {
            Component::Normal(p) => safe.push(p),
            _ => {
                http::send_err(stream, "400 Bad Request", "その場所は配れません");
                return;
            }
        }
    }
    let full = root.join(&safe);
    match std::fs::read(&full) {
        Ok(body) => http::send(stream, "200 OK", http::mime_of(rel), &body),
        Err(_) => http::send(stream, "404 Not Found", "text/plain; charset=utf-8", b"not found"),
    }
}

// ---------------------------------------------------------------- 対戦の口

fn api(stream: &mut TcpStream, rest: &str, req: &http::Request, rooms: &Rooms) {
    match rest {
        // 部屋を立てる
        "create" => {
            let name = http::field(&req.body, "name").unwrap_or("プレイヤー").to_string();
            let players = http::field_num(&req.body, "players").unwrap_or(4).clamp(3, 4) as usize;
            let code = new_code();
            let token = new_token();
            let mut r = rooms.lock().unwrap();
            let mut room = Room::new(code.clone());
            room.members.push(Member { token: token.clone(), name, feeds: Vec::new(), seat: None });
            room.balance(players);
            r.insert(code.clone(), room);
            http::send_json(
                stream,
                &format!("{{\"room\":\"{code}\",\"token\":\"{token}\"}}"),
            );
        }

        // 合言葉で入る
        "join" => {
            let code = http::field(&req.body, "room").unwrap_or("").to_uppercase();
            let name = http::field(&req.body, "name").unwrap_or("プレイヤー").to_string();
            let mut r = rooms.lock().unwrap();
            let Some(room) = r.get_mut(&code) else {
                http::send_err(stream, "404 Not Found", "その合言葉の部屋はありません");
                return;
            };
            if !matches!(room.phase, Phase::Lobby) {
                http::send_err(stream, "409 Conflict", "その部屋はもう始まっています");
                return;
            }
            if room.members.len() >= rooms::MAX_PLAYERS {
                http::send_err(stream, "409 Conflict", "その部屋は満員です");
                return;
            }
            let token = new_token();
            room.members.push(Member { token: token.clone(), name, feeds: Vec::new(), seat: None });
            let want = room.players().max(room.members.len());
            room.balance(want);
            let l = room.lobby_json();
            room.broadcast(&l);
            http::send_json(stream, &format!("{{\"room\":\"{code}\",\"token\":\"{token}\"}}"));
        }

        // 待機所の設定を変える（人数・CPU の強さ）
        "settings" => {
            let mut r = rooms.lock().unwrap();
            let Some(room) = room_of(&mut r, req) else {
                http::send_err(stream, "404 Not Found", "部屋がありません");
                return;
            };
            if let Some(n) = http::field_num(&req.body, "players") {
                room.balance(n.clamp(3, 4) as usize);
            }
            if let (Some(i), Some(lv)) =
                (http::field_num(&req.body, "cpu"), http::field_num(&req.body, "level"))
            {
                if let Some(slot) = room.cpus.get_mut(i.max(0) as usize) {
                    *slot = lv.clamp(0, 3) as u32;
                }
            }
            let l = room.lobby_json();
            room.broadcast(&l);
            http::send_json(stream, "{\"ok\":true}");
        }

        "start" => {
            let mut r = rooms.lock().unwrap();
            let Some(room) = room_of(&mut r, req) else {
                http::send_err(stream, "404 Not Found", "部屋がありません");
                return;
            };
            room.start();
            http::send_json(stream, "{\"ok\":true}");
        }

        // 手を指す。**手番かどうかはここで裁く**
        "act" => {
            let mut r = rooms.lock().unwrap();
            let token = http::field(&req.body, "token").unwrap_or("").to_string();
            let Some(room) = room_of(&mut r, req) else {
                http::send_err(stream, "404 Not Found", "部屋がありません");
                return;
            };
            let Some(mi) = room.member_of(&token) else {
                http::send_err(stream, "403 Forbidden", "その鍵は通りません");
                return;
            };
            let Some(seat) = room.members[mi].seat else {
                http::send_err(stream, "409 Conflict", "まだ席がありません");
                return;
            };
            // 一覧から番号で指す手と、画面で組み立てた手（交易・捨て札）の 2 通り
            let r = if let Some(kind) = http::field(&req.body, "k") {
                let g = http::field_bundle(&req.body, "g").unwrap_or([0; 5]);
                let w = http::field_bundle(&req.body, "w").unwrap_or([0; 5]);
                let n = http::field_num(&req.body, "n").unwrap_or(0);
                let kind = kind.to_string();
                room.apply_custom(seat, &kind, g, w, n)
            } else {
                let i = http::field_num(&req.body, "i").unwrap_or(-1);
                room.apply_index(seat, i.max(0) as usize)
            };
            match r {
                Ok(()) => http::send_json(stream, "{\"ok\":true}"),
                Err(e) => http::send_err(stream, "409 Conflict", e),
            }
        }

        "again" => {
            let mut r = rooms.lock().unwrap();
            let token = http::field(&req.body, "token").unwrap_or("").to_string();
            let Some(room) = room_of(&mut r, req) else {
                http::send_err(stream, "404 Not Found", "部屋がありません");
                return;
            };
            room.vote_again(&token);
            http::send_json(stream, "{\"ok\":true}");
        }

        "home" => {
            let mut r = rooms.lock().unwrap();
            let Some(room) = room_of(&mut r, req) else {
                http::send_err(stream, "404 Not Found", "部屋がありません");
                return;
            };
            room.go_home();
            http::send_json(stream, "{\"ok\":true}");
        }

        // 配信の口。ここだけ繋ぎっぱなしにする
        "events" => {
            let code = req.query.get("room").cloned().unwrap_or_default().to_uppercase();
            let token = req.query.get("token").cloned().unwrap_or_default();
            if !http::open_sse(stream) {
                return;
            }
            let Ok(feed) = stream.try_clone() else { return };
            {
                let mut r = rooms.lock().unwrap();
                let Some(room) = r.get_mut(&code) else { return };
                let Some(mi) = room.member_of(&token) else { return };
                room.members[mi].feeds.push(feed);
                let l = room.lobby_json();
                room.send_to(mi, &l);
            }
            // 繋ぎっぱなしにする。落ちたら retain で外れる
            loop {
                thread::sleep(Duration::from_secs(20));
                let mut r = rooms.lock().unwrap();
                let Some(room) = r.get_mut(&code) else { return };
                if room.member_of(&token).is_none() {
                    return;
                }
                // 途中の機器が切らないよう、時々何か流す
                let alive = room
                    .members
                    .iter_mut()
                    .find(|m| m.token == token)
                    .map(|m| {
                        m.feeds.retain_mut(|f| http::push(f, "{\"t\":\"ping\"}"));
                        !m.feeds.is_empty()
                    })
                    .unwrap_or(false);
                if !alive {
                    return;
                }
            }
        }

        _ => http::send_err(stream, "404 Not Found", "そんな口はありません"),
    }
}

fn room_of<'a>(
    rooms: &'a mut HashMap<String, Room>,
    req: &http::Request,
) -> Option<&'a mut Room> {
    let code = http::field(&req.body, "room")
        .map(|s| s.to_uppercase())
        .or_else(|| req.query.get("room").map(|s| s.to_uppercase()))?;
    rooms.get_mut(&code)
}
