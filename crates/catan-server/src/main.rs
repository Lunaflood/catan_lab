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
                    let alive = room.members.iter().any(|m| m.away_ms() < 60_000);
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
            room.members.push(Member::new(token.clone(), name));
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
            room.members.push(Member::new(token.clone(), name));
            let want = room.players().max(room.members.len());
            room.balance(want);
            room.send_lobby();
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
            room.send_lobby();
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
        // 出来事を取りに来る口。
        //
        // 🔴 **垂れ流し（SSE）は使わない**。Cloudflare の無料トンネルは
        // 長さの決まらない応答を溜め込み、ヘッダだけ届いて本文が 1 バイトも流れない
        // （実測。`Transfer-Encoding: chunked` を付けても、詰め物を 16KB 入れても駄目）。
        // 「1 回の要求に 1 回の応答」なら、どんな中継でも必ず通る。
        //
        // ただの一定間隔の取りに行きにすると、手を指してから相手の画面に出るまでが
        // その間隔ぶん遅れる。そこで**待たせる**（long poll）── 新しい出来事が
        // 出るまで最大 25 秒その場で持ち、出た瞬間に返す。
        // 体感は垂れ流しとほぼ同じで、経路は普通の要求と応答のまま。
        "poll" => {
            let code = req
                .query
                .get("room")
                .cloned()
                .unwrap_or_default()
                .to_uppercase();
            let token = req.query.get("token").cloned().unwrap_or_default();
            let since: u64 = req
                .query
                .get("since")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);

            // 初回（since=0）は、まず待機所の様子を作ってから渡す
            {
                let mut r = rooms.lock().unwrap();
                let Some(room) = r.get_mut(&code) else {
                    http::send_err(stream, "404 Not Found", "部屋がありません");
                    return;
                };
                let Some(mi) = room.member_of(&token) else {
                    http::send_err(stream, "404 Not Found", "その部屋には居ません");
                    return;
                };
                room.members[mi].seen_at = rooms::now_ms();
                if since == 0 {
                    // 初めて（＝読み直した直後かもしれない）。
                    // 対局中なら、作り直すのに要る物を丸ごと渡す
                    room.send_lobby();
                    room.resend_game(mi);
                }
            }

            let deadline = std::time::Instant::now() + Duration::from_secs(25);
            loop {
                {
                    let mut r = rooms.lock().unwrap();
                    let Some(room) = r.get_mut(&code) else {
                        http::send_err(stream, "404 Not Found", "部屋がありません");
                        return;
                    };
                    let Some(mi) = room.member_of(&token) else {
                        http::send_err(stream, "404 Not Found", "その部屋には居ません");
                        return;
                    };
                    room.members[mi].seen_at = rooms::now_ms();
                    if let Some(body) = room.drain_for(mi, since) {
                        http::send_json(stream, &body);
                        return;
                    }
                    if std::time::Instant::now() >= deadline {
                        // 何も無かった。**先端の番号だけ返す**（次はそこから聞く）
                        let seq = room.head_seq(mi).max(since);
                        http::send_json(stream, &format!("{{\"seq\":{seq},\"msgs\":[]}}"));
                        return;
                    }
                }
                thread::sleep(Duration::from_millis(80));
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
