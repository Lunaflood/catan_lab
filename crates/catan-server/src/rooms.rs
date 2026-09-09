//! 部屋と対局の進行。
//!
//! ## なぜサーバが盤面を配らないのか
//!
//! エンジンは決定的なので、**同じ種と同じ手順**を与えれば、どの端末でも必ず同じ盤面になる。
//! だからサーバが配るのは「誰が何番目に何を指したか」だけでよく、
//! 盤面そのものは各自のブラウザが自分で再現する。
//! 盤面を JSON で配る作りに比べて、送る量も書く量も桁違いに少ない。
//!
//! サーバの仕事は 3 つだけ:
//!   ・席を配る
//!   ・**手番でない人の手を撥ねる**（ここだけは信用できないので必ずサーバで裁く）
//!   ・CPU の手を決めて配る（各自が勝手に決めるとズレるため）

use catan_ai::bots::{bot_for_level, Bot};
use catan_core::action::Action;
use catan_core::game::{Game, GameConfig, TurnClock};
use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub const MAX_PLAYERS: usize = 4;

/// ひとこと 1 つの長さの上限（文字数）。画面の幅が壊れない範囲
pub const MAX_SAY_CHARS: usize = 120;
/// 控えておく発言の数。繋ぎ直した人に渡す分
pub const MAX_CHAT_KEPT: usize = 60;
/// 同じ人が続けて発言できる間隔
pub const SAY_INTERVAL_MS: u128 = 600;

/* ---------------------------------------------------------------- 持ち時間

   🔴🔴🔴 **時計はエンジンの外にしか置かない**。

   `GameConfig::turn_time_limit_ms` は既にあるが、それは `negotiation_open()` を
   通じて `legal_actions`（game.rs:581, 671）と `can_apply`（game.rs:1001, 1012）に
   効く。サーバの時計は仮想（1 手 500ms）、端末の時計は実時間なので、
   そこに本物の制限を入れると**手の個数がサーバと端末で食い違う**。
   番号だけを配る方式なので、これは即座に盤の食い違いになる
   （＝海上交易の控え漏れで踏んだのと同じ層の事故）。

   だから `turn_time_limit_ms` は 3 か所すべてで `None` のまま据え置き、
   ここで測るのは**サーバのローカルな締切だけ**。時間切れになったら
   サーバが普通の手として `commit` する。端末から見れば、ただ誰かが指しただけ。   */

/// 場面ごとの持ち時間（ミリ秒）。0 は「無制限」
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Limits {
    /// 初期配置の開拓地
    pub setup_settlement: u32,
    /// 初期配置の道
    pub setup_road: u32,
    /// 盗賊を置く
    pub robber: u32,
    /// サイコロを振るか発展カードを使うか（振る前）
    pub roll: u32,
    /// 手番（振った後）
    pub turn: u32,
    /// 提案への返事
    pub answer: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            setup_settlement: 5 * 60_000,
            setup_road: 60_000,
            robber: 60_000,
            roll: 30_000,
            turn: 4 * 60_000,
            answer: 15_000,
        }
    }
}

impl Limits {
    /// 待機所から来た値を人が遊べる範囲に収める。0 は無制限として通す
    pub fn clamp_one(v: i64) -> u32 {
        if v <= 0 {
            return 0;
        }
        (v as u32).clamp(5_000, 30 * 60_000)
    }

    pub fn as_array(&self) -> [u32; 6] {
        [self.setup_settlement, self.setup_road, self.robber, self.roll, self.turn, self.answer]
    }

    /// 捨て札の持ち時間。盗賊と同じ場面（7 が出た時）なので合わせる
    fn discard(&self) -> u32 { self.robber }
    /// 提案者が相手を選ぶ時間。全員の返事を見てから決めるので返事の 3 倍
    fn acceptees(&self) -> u32 { self.answer.saturating_mul(3) }
    /// 街道建設カードの無償の道。道を置く動作なので初期配置の道と同じ
    fn free_road(&self) -> u32 { self.setup_road }
}

/// 「いまどの場面か」を表す鍵。**これが変わったら締切を引き直す**。
///
/// 中身は `Game::fingerprint()` が読んでいる材料の部分集合なので、
/// 新しい状態も新しい結線も増えない。毎周回で盤から作り直すので、
/// 「ここでも引き直す」の書き忘れが起きない
/// （海上交易の `history.push` 忘れと同じ型の事故を、構造で防ぐ）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SceneKey {
    turn: u32,
    turn_player: u8,
    to_act: u8,
    prompt: u8,
    rolled: bool,
    setup_index: u8,
    dev_played: bool,
    free_roads: u8,
}

/// 取りに来なくなった人を待つ長さ。これを過ぎたら締切を詰める
pub const AWAY_MS: u128 = 45_000;
/// 取りに来ない人に残す猶予
pub const AWAY_GRACE_MS: u128 = 3_000;

/// 参加者ひとり。CPU は `token` を持たない
pub struct Member {
    pub token: String,
    pub name: String,
    /// この人へ渡す出来事の控え。`(通し番号, 中身)`
    ///
    /// 🔴 **垂れ流し（SSE）はやめて、ここに積んで取りに来てもらう**。
    /// Cloudflare の無料トンネルは長さの決まらない応答を溜め込んでしまい、
    /// ヘッダだけ届いて本文が 1 バイトも流れない（実測。詰め物 16KB でも駄目）。
    /// 「1 回の要求に 1 回の応答」なら、どんな中継でも必ず通る。
    pub outbox: Vec<(u64, String)>,
    /// 最後に取りに来た時刻。**居るかどうかの判定はこれで行う**
    pub seen_at: u128,
    pub seat: Option<usize>,
    /// 最後に発言した時刻。連投を抑えるためだけに使う
    pub said_at: u128,
}

impl Member {
    pub fn new(token: String, name: String) -> Self {
        Member { token, name, outbox: Vec::new(), seen_at: now_ms(), seat: None, said_at: 0 }
    }

    /// 取りに来ていない時間。長く空いたら「接続待ち」として出す
    pub fn away_ms(&self) -> u128 {
        now_ms().saturating_sub(self.seen_at)
    }
}

pub enum Phase {
    Lobby,
    Playing,
    Over,
}

pub struct Room {
    pub code: String,
    pub members: Vec<Member>,
    /// CPU の強さ。人間の数と合わせて `players` 人になる
    pub cpus: Vec<u32>,
    pub phase: Phase,
    pub game: Option<Game>,
    pub bots: Vec<Option<Box<dyn Bot>>>,
    pub seeds: [u32; 4],
    /// 席 → 参加者の番号（CPU の席は `None`）
    pub seat_member: Vec<Option<usize>>,
    pub again: HashSet<String>,
    /// 次に CPU を動かしてよい時刻（人が目で追えるよう間を空ける）
    pub bot_at: u128,
    pub last_touch: u128,
    /// 出来事の順序番号（v2 CPU への配信用）
    pub evseq: u64,
    /// 控えの通し番号。取りに来た人が「どこまで受け取ったか」を言えるようにする
    pub msgseq: u64,
    /// この対局で指された手の控え（`act` の電文そのまま）。
    ///
    /// 🔴 **読み直しても対局が消えないため**に持つ。端末が繋ぎ直してきたら、
    /// 開始の合図と一緒にこれを全部渡せば、決定的なエンジンが同じ盤を作り直す。
    pub history: Vec<String>,
    /// 開始の合図（席ごとに中身が違うので人数ぶん持つ）
    pub start_msgs: Vec<String>,
    /// ひとことの控え（直近だけ）。
    ///
    /// 🔴 **`history` に混ぜてはいけない**。`history` は繋ぎ直してきた人の端末が
    /// 「手」として順に自分のエンジンへ入れ直す並びで、そこに手でない物が
    /// 入ると盤が食い違う（＝直前に直したばかりの事故と同じ層）。
    /// だから別の入れ物に持ち、再送も別に行う。
    pub chat: Vec<String>,
    /// 場面ごとの持ち時間。待機所で変えられる
    pub limits: Limits,
    /// いま測っている場面と、その締切（時刻）
    pub deadline: Option<(SceneKey, u128)>,
}

pub fn now_ms() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis()
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// 合言葉と鍵。暗号用途ではないので、時刻と連番を混ぜるだけで足りる
fn rand_hex(n: usize) -> String {
    let mut x = now_ms() as u64 ^ (COUNTER.fetch_add(1, Ordering::SeqCst) << 32);
    let mut s = String::new();
    while s.len() < n {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        s.push_str(&format!("{:016x}", x));
    }
    s.truncate(n);
    s
}

/// 人が読んで打ち込む合言葉。紛らわしい文字（0/O, 1/I）は使わない
pub fn new_code() -> String {
    const A: &[u8] = b"23456789ABCDEFGHJKLMNPQRSTUVWXYZ";
    let h = rand_hex(8);
    h.as_bytes()
        .chunks(2)
        .take(4)
        .map(|c| {
            let v = u8::from_str_radix(std::str::from_utf8(c).unwrap(), 16).unwrap_or(0);
            A[(v as usize) % A.len()] as char
        })
        .collect()
}

pub fn new_token() -> String {
    rand_hex(24)
}

impl Room {
    pub fn new(code: String) -> Self {
        Room {
            code,
            members: Vec::new(),
            cpus: vec![1, 1, 1],
            phase: Phase::Lobby,
            game: None,
            bots: Vec::new(),
            seeds: [0; 4],
            seat_member: Vec::new(),
            again: HashSet::new(),
            bot_at: 0,
            last_touch: now_ms(),
            evseq: 0,
            msgseq: 0,
            history: Vec::new(),
            start_msgs: Vec::new(),
            chat: Vec::new(),
            limits: Limits::default(),
            deadline: None,
        }
    }

    pub fn players(&self) -> usize {
        (self.members.len() + self.cpus.len()).clamp(3, MAX_PLAYERS)
    }

    pub fn member_of(&self, token: &str) -> Option<usize> {
        self.members.iter().position(|m| m.token == token)
    }

    /// 人数を合わせる。人が増えたら CPU を減らし、減ったら足す
    pub fn balance(&mut self, want_players: usize) {
        let want = want_players.clamp(3, MAX_PLAYERS);
        let humans = self.members.len();
        let need_cpu = want.saturating_sub(humans);
        while self.cpus.len() > need_cpu {
            self.cpus.pop();
        }
        while self.cpus.len() < need_cpu {
            self.cpus.push(1);
        }
    }

    // ---------------------------------------------------------------- 配信

    /// 出来事の控えに積む。取りに来た人へ次の `poll` で渡る。
    /// 古い分は捨てる（取りに来ない人の控えが際限なく伸びないように）
    fn queue(&mut self, mi: usize, msg: &str) {
        self.msgseq += 1;
        let id = self.msgseq;
        if let Some(m) = self.members.get_mut(mi) {
            m.outbox.push((id, msg.to_string()));
            if m.outbox.len() > 400 {
                let cut = m.outbox.len() - 400;
                m.outbox.drain(..cut);
            }
        }
    }

    pub fn broadcast(&mut self, msg: &str) {
        for i in 0..self.members.len() {
            self.queue(i, msg);
        }
    }

    pub fn send_to(&mut self, mi: usize, msg: &str) {
        self.queue(mi, msg);
    }

    /// `since` より後の控えを JSON にして返す。無ければ `None`
    pub fn drain_for(&self, mi: usize, since: u64) -> Option<String> {
        let m = self.members.get(mi)?;
        let items: Vec<&str> = m
            .outbox
            .iter()
            .filter(|(id, _)| *id > since)
            .map(|(_, s)| s.as_str())
            .collect();
        if items.is_empty() {
            return None;
        }
        let last = m.outbox.last().map(|(id, _)| *id).unwrap_or(since);
        Some(format!(
            "{{\"seq\":{},\"msgs\":[{}]{}}}",
            last,
            items.join(","),
            self.clock_json()
        ))
    }

    /// 残り時間。**電文ではなく取りに来た応答の封筒に載せる**。
    ///
    /// 電文にすると (1) 控えの枠を手と食い合う (2)「手ではない種類」が増える
    /// という 2 つの困りごとが出る。封筒なら構造的に `history` に入りようがなく、
    /// 取りに来るたびに作り直すので必ず新しい。
    pub fn clock_json(&self) -> String {
        let (Some(left), Some(limit)) = (self.time_left_ms(), self.time_limit_ms()) else {
            return String::new();
        };
        let seat = self
            .game
            .as_ref()
            .map(|g| g.to_act as i32)
            .unwrap_or(-1);
        format!(",\"clock\":{{\"leftMs\":{left},\"limitMs\":{limit},\"seat\":{seat}}}")
    }

    /// 取りこぼしを防ぐため、いまの控えの先端を返す
    pub fn head_seq(&self, mi: usize) -> u64 {
        self.members
            .get(mi)
            .and_then(|m| m.outbox.last().map(|(id, _)| *id))
            .unwrap_or(0)
    }

    /// 待機所の様子。席は開始時に決まるので、ここでは並びだけ知らせる。
    ///
    /// `you` は受け取る人が一覧の何番目か。**誰が自分かは人によって違う**ので、
    /// 全員に同じ一通を配ると「入れているのは自分か友達か」を画面で示せない。
    /// 部屋を立てた人は必ず 0 番（`members` の先頭）。
    pub fn lobby_json_for(&self, you: usize) -> String {
        let names: Vec<String> = self
            .members
            .iter()
            .map(|m| {
                format!(
                    "{{\"name\":\"{}\",\"here\":{}}}",
                    crate::http::esc(&m.name),
                    // 5 秒以内に取りに来ていれば「居る」
                    m.away_ms() < 5000
                )
            })
            .collect();
        let cpus: Vec<String> = self.cpus.iter().map(|l| l.to_string()).collect();
        let l = self.limits.as_array();
        format!(
            "{{\"t\":\"lobby\",\"code\":\"{}\",\"members\":[{}],\"cpus\":[{}],\"players\":{},\"you\":{},\"limits\":[{},{},{},{},{},{}]}}",
            self.code,
            names.join(","),
            cpus.join(","),
            self.players(),
            you,
            l[0], l[1], l[2], l[3], l[4], l[5]
        )
    }

    /// 待機所の様子を全員へ。**一人ずつ違う一通**を配る（`you` が違うため）
    pub fn send_lobby(&mut self) {
        for i in 0..self.members.len() {
            let msg = self.lobby_json_for(i);
            self.send_to(i, &msg);
        }
    }

    // ---------------------------------------------------------------- 対局

    pub fn start(&mut self) {
        // 誰が 1 番手になるかは種で決めるので、部屋を立てた人が有利にはならない
        let seeds: [u32; 4] = [
            (now_ms() as u32) ^ 0x9e37_79b9,
            (now_ms() as u32).wrapping_mul(2654435761) ^ 0x1234_5678,
            (now_ms() as u32).wrapping_add(0x0f0f_0f0f),
            (now_ms() as u32).rotate_left(7) ^ 0xabcd_ef01,
        ];
        self.start_with_seeds(seeds);
    }

    /// 種を指定して始める。**試験から呼ぶために分けてある**
    /// （時計から種を取ると、走らせるたびに別の対局になり、
    /// 落ちたり通ったりする試験になってしまう）
    pub fn start_with_seeds(&mut self, seeds: [u32; 4]) {
        let players = self.players();
        // 席を配る。人が先に座り、残りが CPU。
        let off = (seeds[1] as usize) % players;
        self.seat_member = vec![None; players];
        for (i, _) in self.members.iter().enumerate() {
            self.seat_member[(off + i) % players] = Some(i);
        }
        for (i, m) in self.members.iter_mut().enumerate() {
            m.seat = Some((off + i) % players);
        }

        let mut cfg = GameConfig::default();
        cfg.turn_clock = TurnClock::PerAction { ms: 500 };
        cfg.turn_time_limit_ms = None;
        // 🔴 **端末と同じ値でなければならない**。
        // 交渉で誰から順に聞くかは規則の一部なので、ここがずれると
        // `to_act` がサーバと端末で食い違い、盤面がずれて進行不能になる。
        // 端末側は `game_new` の `ask_last_mask` に同じ物を渡す
        cfg.answer_last_mask = (0..players)
            .filter(|&s| self.seat_member[s].is_some())
            .fold(0u8, |m, s| m | (1 << s));
        let game = Game::with_streams(
            players as u8,
            seeds[0] as u64,
            seeds[1] as u64,
            seeds[2] as u64,
            seeds[3] as u64,
            cfg,
        );

        // CPU は席ごとに用意する。強さは待機所で選ばれたものを順に当てる
        let mut ci = 0usize;
        // 席ごとの強さ。人の席は -1。端末が名前の脇に小さく添えるので配る
        let mut seat_levels: Vec<i32> = vec![-1; players];
        self.bots = (0..players)
            .map(|seat| {
                if self.seat_member[seat].is_some() {
                    None
                } else {
                    let lv = *self.cpus.get(ci).unwrap_or(&1);
                    seat_levels[seat] = lv as i32;
                    ci += 1;
                    // CPU へ本番の乱数の種を渡さない。
                    Some(bot_for_level(lv, 0xC47A_2026 + seat as u64))
                }
            })
            .collect();

        self.seeds = seeds;
        self.game = Some(game);
        self.evseq = 0;
        self.phase = Phase::Playing;
        self.deadline = None;
        self.again.clear();
        self.bot_at = now_ms() + 700;

        // 席ごとに「自分の席」が違うので、1 人ずつ別の内容を送る。
        //
        // CPU には**席ごとの名前**を付ける。強さをそのまま名前にすると
        // 4 人中 3 人が「さいきょう」になり、ログで色を見比べないと呼び分けられない。
        // 端末側（web/colonist/app.js の `CPU_NAMES`）と同じ並びにすること
        const CPU_NAMES: [&str; MAX_PLAYERS] = ["カイ", "ミナ", "レン", "ソラ"];
        let names: Vec<String> = (0..players)
            .map(|seat| match self.seat_member[seat] {
                Some(mi) => crate::http::esc(&self.members[mi].name),
                None => CPU_NAMES[seat.min(MAX_PLAYERS - 1)].to_string(),
            })
            .collect();
        let names_json = names
            .iter()
            .map(|n| format!("\"{n}\""))
            .collect::<Vec<_>>()
            .join(",");
        // どの席が人かも配る。端末側のエンジンは「自分だけが人」として動かすので、
        // これが無いと他の人が CPU として表示されてしまう
        let humans_json = (0..players)
            .map(|seat| if self.seat_member[seat].is_some() { "true" } else { "false" })
            .collect::<Vec<_>>()
            .join(",");
        let levels_json = seat_levels
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join(",");

        let lim = self.limits.as_array();
        let mut msgs = Vec::new();
        for (mi, m) in self.members.iter().enumerate() {
            let seat = m.seat.unwrap_or(0);
            msgs.push((
                mi,
                format!(
                    "{{\"t\":\"start\",\"seeds\":[{},{},{},{}],\"players\":{},\
                     \"yourSeat\":{},\"names\":[{}],\"humans\":[{}],\"levels\":[{}],\"limits\":[{},{},{},{},{},{}]}}",
                    seeds[0], seeds[1], seeds[2], seeds[3], players, seat, names_json, humans_json,
                    levels_json, lim[0], lim[1], lim[2], lim[3], lim[4], lim[5]
                ),
            ));
        }
        // 席ごとの開始の合図は取っておく。繋ぎ直してきた人へもう一度渡すため
        self.history.clear();
        self.start_msgs = vec![String::new(); self.members.len()];
        for (mi, msg) in msgs {
            if let Some(slot) = self.start_msgs.get_mut(mi) {
                *slot = msg.clone();
            }
            self.send_to(mi, &msg);
        }
    }

    /// 繋ぎ直してきた人へ、対局を作り直すのに要る物を全部渡す。
    ///
    /// 盤面は送らない ── エンジンが決定的なので、
    /// **開始の合図＋指された手の並び**があれば端末が同じ盤を作れる。
    pub fn resend_game(&mut self, mi: usize) {
        if !matches!(self.phase, Phase::Playing | Phase::Over) {
            return;
        }
        let start = self.start_msgs.get(mi).cloned().unwrap_or_default();
        if start.is_empty() {
            return;
        }
        self.send_to(mi, &start);
        let hist = self.history.clone();
        for msg in hist {
            self.send_to(mi, &msg);
        }
    }

    /// 手を 1 つ進めて、全員に配る。撥ねたら `Err`。
    ///
    /// `echo` は配る中身。**各自の端末は同じ手を自分のエンジンに入れて盤を再現する**ので、
    /// ここで配るものと、こちらが指したものが食い違ってはいけない。
    fn commit(&mut self, a: Action, echo: String) -> Result<(), &'static str> {
        let Some(g) = self.game.as_mut() else {
            return Err("対局が始まっていない");
        };
        let before = g.clone();
        let rec = g.apply(a);
        self.evseq += 1;
        catan_ai::harness::notify_seats(&mut self.bots, &before, g, &rec, self.evseq);
        let over = g.is_over();
        let turn = g.turn;
        let to_act = g.to_act;
        let fp = g.fingerprint();
        let msg =
            format!("{{\"t\":\"act\",{echo},\"turn\":{turn},\"toAct\":{to_act},\"fp\":{fp}}}");
        self.history.push(msg.clone());
        self.broadcast(&msg);
        if over {
            self.phase = Phase::Over;
            self.broadcast("{\"t\":\"over\"}");
        }
        self.bot_at = now_ms() + 550;
        Ok(())
    }

    /// 手番かどうかを確かめる。**ここだけは端末を信用しない**
    fn check_turn(&self, seat: usize) -> Result<(), &'static str> {
        let Some(g) = self.game.as_ref() else {
            return Err("対局が始まっていない");
        };
        if g.is_over() {
            return Err("もう終わっている");
        }
        if g.to_act as usize != seat {
            return Err("あなたの手番ではない");
        }
        Ok(())
    }

    /// 手番号は局面ごとに意味が変わる。遅延・二重送信を着手前に検出する。
    pub fn check_client_state(&self, expected: Option<u32>) -> Result<(), &'static str> {
        let Some(expected) = expected else {
            return Err("画面を再読み込みしてから操作してください");
        };
        let Some(game) = self.game.as_ref() else { return Err("対局が始まっていない"); };
        if game.fingerprint() != expected {
            return Err("盤面が更新されています。最新の画面で選び直してください");
        }
        Ok(())
    }

    /// 合法手の一覧から番号で指す
    pub fn apply_index(&mut self, seat: usize, index: usize) -> Result<(), &'static str> {
        self.check_turn(seat)?;
        let g = self.game.as_ref().unwrap();
        let acts = g.legal_actions();
        let Some(&a) = acts.get(index) else {
            return Err("その手は無い");
        };
        self.commit(a, format!("\"i\":{index}"))
    }

    /// 画面で組み立てた手（交易の提案・対案・捨て札）。
    /// 一覧に無い形でも、規則として正しければエンジンは受ける
    pub fn apply_custom(
        &mut self,
        seat: usize,
        kind: &str,
        g1: [u8; 5],
        w1: [u8; 5],
        n: i64,
    ) -> Result<(), &'static str> {
        self.check_turn(seat)?;
        let game = self.game.as_ref().unwrap();
        let a = match kind {
            "offer" => Action::OfferTrade { give: g1, want: w1 },
            "counter" => Action::CounterOffer { give: g1, want: w1 },
            "alt" => Action::CounterAlt { give: g1, want: w1 },
            "altRemove" => Action::CounterAltRemove(n.clamp(0, 255) as u8),
            "discard" => Action::Discard(g1),
            // 銀行との交換は「1 回の操作」だが、規則の上では海上交易を何度も指すのと同じ。
            // 端末側の `maritime_bulk` と**同じ順**で指す必要があるので、ここも同じ手順で回す
            "maritime" => return self.apply_maritime(seat, g1, w1),
            _ => return Err("知らない手です"),
        };
        if !game.can_apply(&a) {
            return Err("その条件では指せません");
        }
        // 🔴 対案は「候補を 1 つ積む」を何度も送る**多手順**。
        //    ところが CounterAlt / CounterAltRemove は場面の鍵を 1 つも動かさない
        //    （prompt も to_act も turn も rolled も変わらない）ので、
        //    鍵だけを見ていると 15 秒の間に条件を組み立てきれない。
        //    自分で手を動かしている間は締切を引き直す。
        if matches!(kind, "alt" | "altRemove") {
            self.extend_deadline();
        }
        let echo = match kind {
            "altRemove" => format!("\"k\":\"altRemove\",\"n\":{n}"),
            _ => format!(
                "\"k\":\"{kind}\",\"g\":[{},{},{},{},{}],\"w\":[{},{},{},{},{}]",
                g1[0], g1[1], g1[2], g1[3], g1[4], w1[0], w1[1], w1[2], w1[3], w1[4]
            ),
        };
        self.commit(a, echo)
    }

    /// CPU の手番なら 1 手進める。人が目で追えるように間を空ける
    pub fn step_bot(&mut self) -> bool {
        if !matches!(self.phase, Phase::Playing) || now_ms() < self.bot_at {
            return false;
        }
        let Some(g) = self.game.as_mut() else { return false };
        if g.is_over() {
            return false;
        }
        let seat = g.to_act as usize;
        if self.seat_member.get(seat).copied().flatten().is_some() {
            return false; // 人の手番
        }
        let Some(Some(bot)) = self.bots.get_mut(seat) else { return false };

        let acts = g.legal_actions();
        if acts.is_empty() {
            return false;
        }
        let view = catan_core::view::View::new(g, seat as u8);
        let chosen = bot.decide(&view, &acts);
        let index = acts.iter().position(|a| *a == chosen).unwrap_or(0);
        let a: Action = acts[index];
        let before = g.clone();
        let rec = g.apply(a);
        self.evseq += 1;
        catan_ai::harness::notify_seats(&mut self.bots, &before, g, &rec, self.evseq);
        let over = g.is_over();
        let turn = g.turn;
        let to_act = g.to_act;
        let fp = g.fingerprint();
        let msg = format!(
            "{{\"t\":\"act\",\"i\":{index},\"turn\":{turn},\"toAct\":{to_act},\"fp\":{fp}}}"
        );
        self.history.push(msg.clone());
        self.broadcast(&msg);
        if over {
            self.phase = Phase::Over;
            self.broadcast("{\"t\":\"over\"}");
        }
        // サイコロの演出が終わるまでは次を出さない
        self.bot_at = now_ms() + if matches!(a, Action::Roll) { 1100 } else { 420 };
        true
    }

    /// 銀行・港との交換をまとめて指す。端末側の `maritime_bulk` と同じ判定・同じ順
    fn apply_maritime(&mut self, _seat: usize, give: [u8; 5], take: [u8; 5]) -> Result<(), &'static str> {
        use catan_core::board::RESOURCES;
        let g = self.game.as_ref().unwrap();
        if !matches!(g.prompt, catan_core::action::Prompt::PlayTurn) || !g.rolled {
            return Err("いま交換できません");
        }
        let me = g.to_act;
        let hand = g.players[me as usize].hand;
        let mut trades = 0u32;
        for (i, r) in RESOURCES.iter().enumerate() {
            if give[i] == 0 {
                continue;
            }
            if take[i] > 0 || give[i] > hand[i] {
                return Err("その組み合わせでは交換できません");
            }
            let rate = g.board.best_maritime_rate(me, *r);
            if rate == 0 || give[i] % rate != 0 {
                return Err("枚数がレートに合いません");
            }
            trades += (give[i] / rate) as u32;
        }
        let want: u32 = take.iter().map(|&n| n as u32).sum();
        if trades == 0 || want != trades {
            return Err("出す枚数と貰う枚数が合いません");
        }
        for i in 0..5 {
            if take[i] > g.bank[i] {
                return Err("銀行にその資源がありません");
            }
        }

        let mut queue = Vec::new();
        for (i, r) in RESOURCES.iter().enumerate() {
            for _ in 0..take[i] {
                queue.push(*r);
            }
        }
        let mut qi = 0;
        {
            let g = self.game.as_mut().unwrap();
            for (i, r) in RESOURCES.iter().enumerate() {
                if give[i] == 0 {
                    continue;
                }
                let rate = g.board.best_maritime_rate(me, *r);
                for _ in 0..(give[i] / rate) {
                    let before = g.clone();
                    let rec = g.apply(Action::MaritimeTrade { give: *r, count: rate, take: queue[qi] });
                    self.evseq += 1;
                    catan_ai::harness::notify_seats(&mut self.bots, &before, g, &rec, self.evseq);
                    qi += 1;
                }
            }
        }
        let g = self.game.as_ref().unwrap();
        let (over, turn, to_act, fp) = (g.is_over(), g.turn, g.to_act, g.fingerprint());
        let msg = format!(
            "{{\"t\":\"act\",\"k\":\"maritime\",\"g\":[{},{},{},{},{}],\"w\":[{},{},{},{},{}],\
             \"turn\":{turn},\"toAct\":{to_act},\"fp\":{fp}}}",
            give[0], give[1], give[2], give[3], give[4],
            take[0], take[1], take[2], take[3], take[4]
        );
        // 🔴 **控えに残す**。ここを忘れると、繋ぎ直してきた人へ渡す並びから
        // 海上交易だけが抜け落ちる。その端末は以降ずっと別の盤を見ることになり、
        // しかも手番は合ったままなので**何も表示されずにずれる**
        // （症状: 自分の数字が出たのに資源が来ない・ログにも出ない）。
        self.history.push(msg.clone());
        self.broadcast(&msg);
        if over {
            self.phase = Phase::Over;
            self.broadcast("{\"t\":\"over\"}");
        }
        self.bot_at = now_ms() + 550;
        Ok(())
    }

    // ---------------------------------------------------------------- 持ち時間

    /// いまの場面の鍵。対局中でなければ `None`
    fn scene_key(&self) -> Option<SceneKey> {
        let g = self.game.as_ref()?;
        if g.is_over() {
            return None;
        }
        use catan_core::action::Prompt::*;
        let prompt = match g.prompt {
            SetupSettlement => 0,
            SetupRoad => 1,
            PlayTurn => 2,
            Discard => 3,
            MoveRobber => 4,
            FreeRoad => 5,
            DecideTrade => 6,
            DecideAcceptees => 7,
            GameOver => return None,
        };
        Some(SceneKey {
            turn: g.turn,
            turn_player: g.turn_player,
            to_act: g.to_act,
            prompt,
            rolled: g.rolled,
            setup_index: g.setup_index,
            // 発展カードを振る前に使ったら、振るまでの時間を仕切り直す
            dev_played: g.dev_played_this_turn,
            // 無償の道は 2 本を別々に数える（2 本目が 1 本目と分け合わない）
            free_roads: g.free_roads,
        })
    }

    /// その場面に与える長さ。0 なら無制限
    fn limit_of(&self, k: &SceneKey) -> u32 {
        let l = &self.limits;
        match k.prompt {
            0 => l.setup_settlement,
            1 => l.setup_road,
            2 => {
                if k.rolled {
                    l.turn
                } else {
                    l.roll
                }
            }
            3 => l.discard(),
            4 => l.robber,
            5 => l.free_road(),
            6 => l.answer,
            7 => l.acceptees(),
            _ => 0,
        }
    }

    /// いま指す番の席が人かどうか（CPU の席は `step_bot` が動かすので測らない）
    fn human_member_to_act(&self) -> Option<usize> {
        let g = self.game.as_ref()?;
        self.seat_member.get(g.to_act as usize).copied().flatten()
    }

    /// 締切までの残り（ミリ秒）。測っていないなら `None`
    pub fn time_left_ms(&self) -> Option<u32> {
        let (_, at) = self.deadline?;
        Some(at.saturating_sub(now_ms()).min(u32::MAX as u128) as u32)
    }

    /// いま測っている場面の持ち時間
    pub fn time_limit_ms(&self) -> Option<u32> {
        let (k, _) = self.deadline.as_ref()?;
        Some(self.limit_of(k))
    }

    /// 場面が変わっていれば締切を引き直す。**毎周回で呼ぶ**。
    /// 時刻は外から渡す（エンジンと同じ考え方。試験が実時間を待たずに済む）
    fn refresh_deadline(&mut self, now: u128) {
        let Some(k) = self.scene_key() else {
            self.deadline = None;
            return;
        };
        // 人の席でなければ測らない
        let Some(mi) = self.human_member_to_act() else {
            self.deadline = None;
            return;
        };
        let limit = self.limit_of(&k);
        if limit == 0 {
            self.deadline = None;
            return;
        }
        let same = matches!(self.deadline, Some((old, _)) if old == k);
        if !same {
            self.deadline = Some((k, now + limit as u128));
        } else if let Some((_, at)) = &mut self.deadline {
            // 待機所で長さを縮めたら、いまの場面にもすぐ効かせる
            // （そうしないと「変えたのに効かない」場面が 1 つ残る）
            let cap = now + limit as u128;
            if *at > cap {
                *at = cap;
            }
        }
        // 取りに来なくなった人は待たない。対局が何十分も止まるのを防ぐ
        let away = self.members.get(mi).map(|m| m.away_ms()).unwrap_or(0);
        if away > AWAY_MS {
            let cut = now + AWAY_GRACE_MS;
            if let Some((_, at)) = &mut self.deadline {
                if *at > cut {
                    *at = cut;
                }
            }
        }
    }

    /// 場面を変えない手（対案の積み下ろし）を指した時に、締切を戻す。
    ///
    /// 対案は「候補として積む」を 1 つずつ送る多手順なので、
    /// 鍵だけを見ていると 15 秒の間に条件を組み立てきれない。
    /// 引き直すのは残りが減っている時だけ（伸ばし続けはしない）。
    fn extend_deadline(&mut self) {
        let Some((k, at)) = self.deadline else { return };
        let limit = self.limit_of(&k) as u128;
        if limit == 0 {
            return;
        }
        let want = now_ms() + limit;
        if want > at {
            self.deadline = Some((k, want));
        }
    }

    /// 時間切れなら既定の手を指す。指したら true。**掃引スレッドから呼ぶ**
    pub fn step_deadline(&mut self) -> bool {
        self.step_deadline_at(now_ms())
    }

    /// 時刻を外から渡す版。試験が実時間を待たずに回せるようにするため
    pub fn step_deadline_at(&mut self, now: u128) -> bool {
        if !matches!(self.phase, Phase::Playing) {
            self.deadline = None;
            return false;
        }
        self.refresh_deadline(now);
        let Some((_, at)) = self.deadline else { return false };
        if now < at {
            return false;
        }
        // サイコロの演出の途中では割り込まない（振った直後に手番が飛ぶと読めない）
        if now < self.bot_at {
            return false;
        }
        let Some(a) = self.default_action() else {
            // 指せる手が無い場面。測るのをやめる
            self.deadline = None;
            return false;
        };
        let g = self.game.as_ref().unwrap();
        let acts = g.legal_actions();
        let Some(i) = acts.iter().position(|x| *x == a) else {
            self.deadline = None;
            return false;
        };
        // 🔴 普通の手として配る。history にも指紋にも載るので、
        //    端末から見れば「誰かが指した」のと区別が付かない＝盤はズレない
        let _ = self.commit(a, format!("\"auto\":true,\"i\":{i}"));
        true
    }

    /// 時間切れで指す手。**必ず `legal_actions` の中から選ぶ**
    fn default_action(&self) -> Option<Action> {
        let g = self.game.as_ref()?;
        let acts = g.legal_actions();
        if acts.is_empty() {
            return None;
        }
        use catan_core::action::Prompt::*;
        match g.prompt {
            // 一番よく採れる場所へ置く。賢くしすぎると席を立つ方が得になる
            SetupSettlement => acts
                .iter()
                .filter_map(|a| match a {
                    Action::SetupSettlement(n) => Some((*n, node_pip_sum(g, *n))),
                    _ => None,
                })
                // 同点は小さい番号（決定的にする）
                .max_by_key(|(n, p)| (*p, std::cmp::Reverse(*n)))
                .map(|(n, _)| Action::SetupSettlement(n)),
            // 道はどこでも大差ない。一覧の先頭で決定的に選ぶ
            SetupRoad | FreeRoad | MoveRobber => acts.first().copied(),
            PlayTurn => {
                if !g.rolled {
                    acts.iter().find(|a| matches!(a, Action::Roll)).copied()
                } else {
                    acts.iter().find(|a| matches!(a, Action::EndTurn)).copied()
                }
            }
            // 捨てた後の手札が一番平らになる組み合わせ（偏りを残さない）
            Discard => {
                let hand = g.players[g.to_act as usize].hand;
                acts.iter()
                    .filter(|a| matches!(a, Action::Discard(_)))
                    .min_by_key(|a| {
                        let Action::Discard(b) = a else {
                            return u32::MAX;
                        };
                        (0..5)
                            .map(|r| {
                                let left = hand[r].saturating_sub(b[r]) as u32;
                                left * left
                            })
                            .sum::<u32>()
                    })
                    .copied()
            }
            DecideTrade => acts.iter().find(|a| matches!(a, Action::RejectTrade)).copied(),
            DecideAcceptees => acts.iter().find(|a| matches!(a, Action::CancelTrade)).copied(),
            GameOver => None,
        }
    }

    // ---------------------------------------------------------------- ひとこと

    /// 発言を 1 つ受けて全員へ配る。
    ///
    /// 手ではないので `history` には積まない（積むと繋ぎ直した端末が
    /// これを手として食おうとして盤が壊れる）。代わりに `chat` に控え、
    /// 繋ぎ直してきた人には [`Room::resend_chat`] で別に渡す。
    pub fn say(&mut self, token: &str, text: &str) -> Result<(), &'static str> {
        let Some(mi) = self.member_of(token) else {
            return Err("その鍵は通りません");
        };
        // 空白だけの発言は捨てる。長すぎる物は切る（画面が壊れるのを防ぐ）
        let text: String = text.trim().chars().take(MAX_SAY_CHARS).collect();
        if text.is_empty() {
            return Err("空です");
        }
        // 連投よけ。速すぎる分は黙って捨てる（撥ねると画面に赤字が出て煩い）
        let now = now_ms();
        {
            let m = &mut self.members[mi];
            if now.saturating_sub(m.said_at) < SAY_INTERVAL_MS {
                return Ok(());
            }
            m.said_at = now;
        }
        let (name, seat) = {
            let m = &self.members[mi];
            (crate::http::esc(&m.name), m.seat)
        };
        let seat = seat.map(|s| s.to_string()).unwrap_or_else(|| "null".into());
        // ⚠ ここで潰すのは JSON の記号だけ。**山括弧はそのまま通る**ので、
        //   画面に出す側が必ず文字として逃がすこと（app.js の escapeHtml）
        let msg = format!(
            "{{\"t\":\"chat\",\"who\":{mi},\"seat\":{seat},\"name\":\"{name}\",\"text\":\"{}\"}}",
            crate::http::esc(&text)
        );
        self.chat.push(msg.clone());
        if self.chat.len() > MAX_CHAT_KEPT {
            let cut = self.chat.len() - MAX_CHAT_KEPT;
            self.chat.drain(..cut);
        }
        self.broadcast(&msg);
        Ok(())
    }

    /// 繋ぎ直してきた人へ、直近の発言を渡し直す
    pub fn resend_chat(&mut self, mi: usize) {
        for msg in self.chat.clone() {
            self.send_to(mi, &msg);
        }
    }

    /// 「もう一度遊ぶ」。**全員が押したら**進む
    pub fn vote_again(&mut self, token: &str) {
        self.again.insert(token.to_string());
        let need = self.members.len();
        let have = self.again.len();
        self.broadcast(&format!(
            "{{\"t\":\"vote\",\"again\":{have},\"need\":{need}}}"
        ));
        if have >= need && need > 0 {
            self.start();
        }
    }

    /// 「ホームに戻る」。**誰か 1 人が押したら**全員戻る
    pub fn go_home(&mut self) {
        self.phase = Phase::Lobby;
        self.game = None;
        self.again.clear();
        for m in &mut self.members {
            m.seat = None;
        }
        self.broadcast("{\"t\":\"home\"}");
        self.send_lobby();
    }
}

/// その頂点に面した陸ヘクスの pip（出やすさ）の合計
fn node_pip_sum(g: &Game, n: catan_core::topology::NodeId) -> u32 {
    let topo = catan_core::topology::Topology::get();
    topo.node_tiles[n as usize]
        .as_slice()
        .iter()
        .map(|&t| g.board.tile_pips(t) as u32)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use catan_core::action::Prompt;
    use catan_core::board::RESOURCES;
    use catan_core::rng::Rng;

    /// 端末の代わり。**開始の合図と控えの並びだけ**を頼りに盤を作り直す。
    /// 本物の端末（web/colonist/app.js の `netApply`）と同じことをする
    struct Client {
        game: Game,
        /// 直前に番号を当てた時の一覧。サーバ側と丸ごと比べるために控える
        last_acts: Vec<Action>,
    }

    impl Client {
        fn new(room: &Room) -> Client {
            let players = room.seat_member.len();
            let mut cfg = GameConfig::default();
            // 🔴 **本物のブラウザと同じ設定にする**。
            //    ここを PerAction にしていた頃は、誰も動かしていない構成を
            //    検査していた（＝壊れた計器）。端末は wasm lib.rs:137 で Realtime。
            cfg.turn_clock = TurnClock::Realtime;
            cfg.turn_time_limit_ms = None;
            cfg.answer_last_mask = (0..players)
                .filter(|&s| room.seat_member[s].is_some())
                .fold(0u8, |m, s| m | (1 << s));
            let s = room.seeds;
            Client {
                game: Game::with_streams(
                    players as u8,
                    s[0] as u64,
                    s[1] as u64,
                    s[2] as u64,
                    s[3] as u64,
                    cfg,
                ),
                last_acts: Vec::new(),
            }
        }

        fn replay(&mut self, msg: &str) {
            // ブラウザは 0.5 秒ごとに実時間を流し込む（app.js の setInterval）。
            // 時計が規則に漏れていたら、ここで盤が割れて指紋の照合が落ちる
            self.game.tick(500);
            if let Some(rest) = msg.split("\"k\":\"maritime\"").nth(1) {
                let g = nums(rest, "\"g\":[");
                let w = nums(rest, "\"w\":[");
                let mut queue = Vec::new();
                for (i, r) in RESOURCES.iter().enumerate() {
                    for _ in 0..w[i] {
                        queue.push(*r);
                    }
                }
                let mut qi = 0;
                for (i, r) in RESOURCES.iter().enumerate() {
                    if g[i] == 0 {
                        continue;
                    }
                    let me = self.game.to_act;
                    let rate = self.game.board.best_maritime_rate(me, *r);
                    for _ in 0..(g[i] / rate) {
                        self.game.apply(Action::MaritimeTrade {
                            give: *r,
                            count: rate,
                            take: queue[qi],
                        });
                        qi += 1;
                    }
                }
            } else {
                let i: usize = msg
                    .split("\"i\":")
                    .nth(1)
                    .and_then(|s| s.split(|c: char| !c.is_ascii_digit()).next())
                    .and_then(|s| s.parse().ok())
                    .expect("番号のある手のはず");
                let acts = self.game.legal_actions();
                assert!(i < acts.len(), "配られた番号が端末の一覧に無い");
                self.last_acts = acts.clone();
                self.game.apply(acts[i]);
            }
            // サーバが電文に添えた指紋と、いま作った盤の指紋が合うこと
            if let Some(fp) = msg.split("\"fp\":").nth(1) {
                let fp: u32 = fp
                    .split(|c: char| !c.is_ascii_digit())
                    .next()
                    .unwrap()
                    .parse()
                    .unwrap();
                assert_eq!(self.game.fingerprint(), fp, "指紋が食い違った: {msg}");
            } else {
                panic!("指紋の無い電文: {msg}");
            }
        }
    }

    fn nums(s: &str, key: &str) -> [u8; 5] {
        let body = s.split(key).nth(1).unwrap().split(']').next().unwrap();
        let mut out = [0u8; 5];
        for (i, part) in body.split(',').enumerate().take(5) {
            out[i] = part.trim().parse().unwrap_or(0);
        }
        out
    }

    /// 人 1 人 + CPU で 1 局進める。人の手は乱数で選び、
    /// 交換できる時は**海上交易**（別経路）も混ぜる。
    /// 戻り値は (部屋, 海上交易をした回数)
    fn play(seed: u64) -> (Room, usize) {
        let mut room = Room::new("TEST".into());
        room.members.push(Member::new("tok".into(), "私".into()));
        room.balance(4);
        room.start_with_seeds([
            seed as u32,
            (seed >> 7) as u32 ^ 0x1234_5678,
            (seed >> 13) as u32 ^ 0x0f0f_0f0f,
            (seed >> 19) as u32 ^ 0xabcd_ef01,
        ]);
        let seat = room.members[0].seat.unwrap();
        let mut rng = Rng::with_stream(seed, 99);
        let mut trades = 0usize;

        for _ in 0..4000 {
            room.bot_at = 0;
            if room.step_bot() {
                continue;
            }
            let g = room.game.as_ref().unwrap();
            if g.is_over() || g.to_act as usize != seat {
                break;
            }
            // 3 回に 1 回くらいは海上交易を試す
            if matches!(g.prompt, Prompt::PlayTurn) && g.rolled && rng.below(3) == 0 {
                let hand = g.players[seat].hand;
                let mut plans = Vec::new();
                for (i, _r) in RESOURCES.iter().enumerate() {
                    let rate = g.board.best_maritime_rate(seat as u8, RESOURCES[i]);
                    if rate == 0 || hand[i] < rate {
                        continue;
                    }
                    let Some(take) = (0..5).find(|&k| k != i && g.bank[k] > 0) else { continue };
                    let mut give = [0u8; 5];
                    give[i] = rate;
                    let mut want = [0u8; 5];
                    want[take] = 1;
                    plans.push((give, want));
                }
                let mut done = false;
                for (give, want) in plans {
                    if room.apply_custom(seat, "maritime", give, want, 0).is_ok() {
                        trades += 1;
                        done = true;
                        break;
                    }
                }
                if done {
                    continue;
                }
            }
            let acts = room.game.as_ref().unwrap().legal_actions();
            if acts.is_empty() {
                break;
            }
            let i = rng.below(acts.len() as u32) as usize;
            if room.apply_index(seat, i).is_err() {
                break;
            }
        }
        (room, trades)
    }

    /// 🔴 実際に起きた不具合の再現。
    ///
    /// 海上交易（銀行・港との交換）だけが控えに残っていなかったため、
    /// 読み直した端末は**その分を抜かした並び**で盤を作り直していた。
    /// 手番は合ったままずれるので、画面には何も出ない。
    /// 症状は「自分の数字が出たのに資源が来ない・ログにも出ない」。
    #[test]
    fn 繋ぎ直した端末はサーバと同じ盤になる() {
        let mut total_trades = 0usize;
        let mut moves = 0usize;
        for seed in 0..30u64 {
            let (room, trades) = play(1_000 + seed);
            total_trades += trades;
            moves += room.history.len();
            // 繋ぎ直してきた端末が、控えだけで同じ盤を作れること。
            // `replay` は 1 手ごとに指紋も突き合わせるので、
            // どの手でずれたのかまで分かる
            let mut c = Client::new(&room);
            for msg in room.history.clone() {
                c.replay(&msg);
            }
            assert_eq!(
                c.game.fingerprint(),
                room.game.as_ref().unwrap().fingerprint(),
                "seed {seed}: 繋ぎ直した端末の盤がサーバと違う"
            );
        }
        println!("30 局 / 配った手 {moves} / うち海上交易 {total_trades}");
        assert!(total_trades > 20, "海上交易を通っていない（試験になっていない）");
    }

    /// 対局を 1 つ JSON に書き出す。**本物の wasm でなぞり直す試験**に渡すため。
    /// `CATAN_DUMP` にファイル名を入れて走らせた時だけ書く
    #[test]
    fn 端末での照合用に対局を書き出す() {
        let Ok(path) = std::env::var("CATAN_DUMP") else { return };
        // 海上交易（不具合のあった経路）を十分に含む対局を選ぶ
        let (room, trades) = (0..60u64)
            .map(play)
            .find(|(_, t)| *t >= 4)
            .expect("海上交易を 4 回以上含む対局が見つからない");
        let s = room.seeds;
        let humans: Vec<String> = (0..room.seat_member.len())
            .map(|i| if room.seat_member[i].is_some() { "true" } else { "false" }.to_string())
            .collect();
        let body = format!(
            "{{\"seeds\":[{},{},{},{}],\"players\":{},\"humans\":[{}],\"trades\":{},\"msgs\":[{}]}}",
            s[0], s[1], s[2], s[3],
            room.seat_member.len(),
            humans.join(","),
            trades,
            room.history.join(",")
        );
        std::fs::write(&path, body).unwrap();
        println!("書き出した: {path}（手 {} / 海上交易 {trades}）", room.history.len());
    }

    /// 🔴 番号方式の**本当の前提**: サーバと端末で合法手の一覧が
    /// 並びも個数も一致すること。ここが割れると、次に配られる番号が
    /// 別の手を指す（＝盤が静かにズレる）。
    ///
    /// これまでは `i < acts.len()` しか見ていなかったので、
    /// 一覧がずれていても範囲内なら通ってしまっていた。
    #[test]
    fn サーバと端末の合法手は並びも個数も一致する() {
        let mut steps = 0usize;
        for seed in 0..12u64 {
            let (room, _) = play(3_000 + seed);
            let mut c = Client::new(&room);
            let mut server = Client::new(&room);
            for msg in room.history.clone() {
                c.replay(&msg);
                server.replay(&msg);
                let a = c.game.legal_actions();
                let b = server.game.legal_actions();
                assert_eq!(a.len(), b.len(), "seed {seed}: 合法手の個数が食い違った");
                assert_eq!(a, b, "seed {seed}: 合法手の並びが食い違った");
                steps += 1;
            }
        }
        println!("{steps} 手で一覧を突き合わせた");
        assert!(steps > 2000);
    }

    /// 🔴 この機能の一番の目的: **人が席を立っても対局が前へ進む**。
    ///
    /// 人の席が 1 手も指さないまま、時間切れの自動着手だけで
    /// 決着まで行けることを確かめる。どこか 1 つの場面に出口が無いと
    /// ここで止まる。
    #[test]
    fn 誰も指さなくても対局は最後まで進む() {
        for seed in 0..6u64 {
            let mut room = Room::new("TEST".into());
            room.members.push(Member::new("tok".into(), "私".into()));
            room.balance(4);
            // 待たずに済むよう、持ち時間を最短にする
            room.limits = Limits {
                setup_settlement: 1,
                setup_road: 1,
                robber: 1,
                roll: 1,
                turn: 1,
                answer: 1,
            };
            room.start_with_seeds([
                (7_000 + seed) as u32,
                (seed as u32) ^ 0x1234_5678,
                (seed as u32) ^ 0x0f0f_0f0f,
                (seed as u32) ^ 0xabcd_ef01,
            ]);
            let mut moves = 0usize;
            let mut stuck = false;
            // 仮想の時刻を進める。実時間を待つ必要はない
            let mut now = now_ms();
            for _ in 0..6000 {
                room.bot_at = 0;
                if room.game.as_ref().map_or(true, |g| g.is_over()) {
                    break;
                }
                // 人の席は自動着手、CPU の席は普段どおり
                if room.step_deadline_at(now) || room.step_bot() {
                    moves += 1;
                    continue;
                }
                // 何も起きないなら時刻を進める（締切まで待つ）
                now += 1_000;
                if now > now_ms() + 10 * 60 * 60 * 1000 {
                    stuck = true;
                    break;
                }
            }
            assert!(!stuck, "seed {seed}: 誰も指せずに止まった（{moves} 手目）");
            let g = room.game.as_ref().unwrap();
            assert!(g.is_over(), "seed {seed}: {moves} 手動いたが決着しなかった");
        }
    }

    /// 配る電文には必ず指紋が入っていること。
    /// 入っていない経路があると、そこだけ黙ってずれる余地が残る
    #[test]
    fn 配る手には必ず指紋が付く() {
        let (room, _) = play(777);
        assert!(room.history.len() > 20);
        for msg in &room.history {
            assert!(msg.contains("\"fp\":"), "指紋の無い電文がある: {msg}");
        }
    }

    #[test]
    fn stale_or_duplicate_input_is_rejected_before_reinterpreting_its_index() {
        let mut room = Room::new("TEST".into());
        room.members.push(Member::new("tok".into(), "私".into()));
        room.balance(4);
        room.start_with_seeds([42, 4, 3, 2]);
        let fp = room.game.as_ref().unwrap().fingerprint();
        assert!(room.check_client_state(None).is_err());
        assert!(room.check_client_state(Some(fp)).is_ok());
        room.apply_index(0, 0).unwrap(); // 同じ人が次に道を置くため、手番チェックだけでは連打を防げない。
        let after = room.game.as_ref().unwrap().fingerprint();
        assert_ne!(fp, after);
        assert!(room.check_client_state(Some(fp)).is_err());
        assert_eq!(room.game.as_ref().unwrap().fingerprint(), after);
        assert!(room.check_client_state(Some(after)).is_ok());
    }

}
