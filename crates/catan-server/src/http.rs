//! 最小限の HTTP/1.1。外部の依存を入れない方針なので手で書く。
//!
//! 必要なのは 3 つだけ:
//!   ・静的ファイルを配る
//!   ・小さな JSON を受け取って返す
//!   ・**繋ぎっぱなしにして書き足す**（SSE。オンライン対戦の配信に使う）
//!
//! ⛔ 汎用のサーバにはしない。想定は「身内で遊ぶ間だけ立てる」用途。

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;

pub struct Request {
    pub method: String,
    pub path: String,
    pub query: HashMap<String, String>,
    pub body: String,
}

/// 1 本の要求を読む。読み切れなければ `None`
pub fn read_request(stream: &TcpStream) -> Option<Request> {
    let mut r = BufReader::new(stream.try_clone().ok()?);
    let mut line = String::new();
    if r.read_line(&mut line).ok()? == 0 {
        return None;
    }
    let mut it = line.split_whitespace();
    let method = it.next()?.to_string();
    let target = it.next()?.to_string();

    let mut len = 0usize;
    loop {
        let mut h = String::new();
        if r.read_line(&mut h).ok()? == 0 {
            break;
        }
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some(v) = h.strip_prefix("Content-Length:") {
            len = v.trim().parse().unwrap_or(0);
        }
    }

    let mut body = vec![0u8; len];
    if len > 0 && r.read_exact(&mut body).is_err() {
        return None;
    }

    let (path, qs) = match target.split_once('?') {
        Some((p, q)) => (p.to_string(), q.to_string()),
        None => (target, String::new()),
    };
    let mut query = HashMap::new();
    for kv in qs.split('&').filter(|s| !s.is_empty()) {
        let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
        query.insert(url_decode(k), url_decode(v));
    }

    Some(Request {
        method,
        path,
        query,
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}

fn url_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => {
                let h = |c: u8| (c as char).to_digit(16).unwrap_or(0) as u8;
                out.push(h(b[i + 1]) * 16 + h(b[i + 2]));
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn send(stream: &mut TcpStream, status: &str, ctype: &str, body: &[u8]) {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}

pub fn send_json(stream: &mut TcpStream, body: &str) {
    send(stream, "200 OK", "application/json; charset=utf-8", body.as_bytes());
}

pub fn send_err(stream: &mut TcpStream, status: &str, msg: &str) {
    send(stream, status, "application/json; charset=utf-8",
         format!("{{\"error\":\"{}\"}}", esc(msg)).as_bytes());
}

/// SSE の口を開ける。ここから先は `push` で書き足していく
///
/// 🔴 **`Transfer-Encoding: chunked` を必ず付ける**。
///
/// 長さの分からない本文を `keep-alive` で流すのに区切りを書かないと、HTTP としては
/// 「どこで本文が終わるのか誰にも分からない」電文になる。ブラウザに直結している間は
/// 接続が閉じるまで読んでくれるので**動いてしまう**が、間に中継（cloudflared など）が
/// 入ると、中継は本文の終わりを待ち続けて何も転送しない。結果、
/// **家の中では動くのに外の友達とだけ繋がらない**。実際にこれで
/// 「参加者が見えない・はじめるを押しても始まらない」が起きた。
pub fn open_sse(stream: &mut TcpStream) -> bool {
    let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream; charset=utf-8\r\n\
                Cache-Control: no-store\r\nConnection: keep-alive\r\n\
                Transfer-Encoding: chunked\r\n\
                X-Accel-Buffering: no\r\n\r\n";
    if !(stream.write_all(head.as_bytes()).is_ok() && stream.flush().is_ok()) {
        return false;
    }
    // 🔴 **冒頭に詰め物を流す**。
    //
    // 中継（Cloudflare など）は、応答の頭が小さいうちは溜め込んで転送を始めない。
    // 枠組み（chunked）を正しくしても、**溜め込みは別の話**で、
    // ヘッダだけ届いて本文が 1 バイトも来ない状態になる（実測で 10 秒待っても無音）。
    // 16KB ほど先に押し込むと緩衝が溢れて流れ出す。
    // `:` で始まる行は SSE の注釈なので、受け取る側は読み飛ばす。
    let pad = format!(":{}\n\n", " ".repeat(16384));
    let chunk = format!("{:X}\r\n{}\r\n", pad.len(), pad);
    stream.write_all(chunk.as_bytes()).is_ok() && stream.flush().is_ok()
}

pub fn push(stream: &mut TcpStream, data: &str) -> bool {
    // 改行を含むと SSE の区切りと衝突する。1 行の JSON しか送らない約束にする
    let msg = format!("data: {}\n\n", data.replace('\n', " "));
    // 塊の大きさは**バイト数**（名前に日本語が入ると文字数とはずれる）
    let chunk = format!("{:X}\r\n{}\r\n", msg.len(), msg);
    stream.write_all(chunk.as_bytes()).is_ok() && stream.flush().is_ok()
}

/// JSON の文字列に入れてよい形に直す
pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// 受け取る JSON はこちらが決めた形しか来ない。素朴に値だけ拾う。
/// ⚠ 汎用のパーサではない（入れ子や配列は扱わない）
/// 本文から 1 つの値を取り出す。**JSON の逃げ（\\" や \\n）を正しく解く**。
///
/// 🔴 素朴に「次の " まで」で切ると、`"text":"彼は\\"はい\\"と言った"` のような
/// 値が **途中で切れる**。ひとことは人が自由に打つ文字列なので、
/// 引用符も改行も普通に入ってくる。
pub fn field(body: &str, key: &str) -> Option<String> {
    let pat = format!("\"{key}\"");
    let i = body.find(&pat)? + pat.len();
    let rest = &body[i..];
    let c = rest.find(':')? + 1;
    let rest = rest[c..].trim_start();
    let Some(r) = rest.strip_prefix('"') else {
        // 裸の値（数や true）。区切りまで
        let end = rest
            .find(|c: char| c == ',' || c == '}' || c.is_whitespace())
            .unwrap_or(rest.len());
        return Some(rest[..end].to_string());
    };
    let mut out = String::new();
    let mut it = r.chars();
    while let Some(ch) = it.next() {
        match ch {
            '"' => return Some(out),
            '\\' => match it.next()? {
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                '/' => out.push('/'),
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                'b' => out.push('\u{8}'),
                'f' => out.push('\u{c}'),
                'u' => {
                    let hex: String = it.by_ref().take(4).collect();
                    let v = u32::from_str_radix(&hex, 16).ok()?;
                    // 代理対（絵文字など）は下側だけ来ても壊さない
                    match char::from_u32(v) {
                        Some(c2) => out.push(c2),
                        None => out.push('\u{fffd}'),
                    }
                }
                other => out.push(other),
            },
            c2 => out.push(c2),
        }
    }
    // 閉じの " が無い＝壊れた本文
    None
}

/// `"g":[1,0,2,0,0]` のような 5 要素の配列だけ拾う（資源の束）
pub fn field_bundle(body: &str, key: &str) -> Option<[u8; 5]> {
    let pat = format!("\"{key}\"");
    let i = body.find(&pat)? + pat.len();
    let rest = &body[i..];
    let a = rest.find('[')? + 1;
    let b = rest.find(']')?;
    let mut out = [0u8; 5];
    for (k, part) in rest[a..b].split(',').enumerate() {
        if k >= 5 {
            return None;
        }
        out[k] = part.trim().parse().ok()?;
    }
    Some(out)
}

pub fn field_num(body: &str, key: &str) -> Option<i64> {
    field(body, key)?.parse().ok()
}

pub fn mime_of(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "wasm" => "application/wasm",
        "json" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 🔴 ひとことは人が自由に打つ文字列。引用符も改行も普通に入る。
    /// 素朴に「次の " まで」で切っていた頃は、そこで**文が切れていた**
    #[test]
    fn 逃がした文字を含む値を最後まで取れる() {
        let body = r#"{"room":"AB12","text":"彼は\"はい\"と言った\n改行も\\逆斜線も","n":3}"#;
        assert_eq!(field(body, "room").as_deref(), Some("AB12"));
        assert_eq!(
            field(body, "text").as_deref(),
            Some("彼は\"はい\"と言った\n改行も\\逆斜線も")
        );
        assert_eq!(field(body, "n").as_deref(), Some("3"));
    }

    #[test]
    fn 逃がした値を書き出して読み直すと元に戻る() {
        for src in [
            "ふつうの文",
            "引用符 \" を含む",
            "逆斜線 \\ と改行\nと tab\t",
            "絵文字 🎲 と記号 <b>&amp;</b>",
            "\"}{\"room\":\"HACK\",\"text\":\"割り込み",
        ] {
            let body = format!("{{\"text\":\"{}\"}}", esc(src));
            assert_eq!(field(&body, "text").as_deref(), Some(src), "元に戻らない: {src}");
        }
    }

    #[test]
    fn 閉じていない値は撥ねる() {
        assert_eq!(field(r#"{"text":"切れている"#, "text"), None);
    }
}
