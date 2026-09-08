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
}

impl Member {
    pub fn new(token: String, name: String) -> Self {
        Member { token, name, outbox: Vec::new(), seen_at: now_ms(), seat: None }
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
        Some(format!("{{\"seq\":{},\"msgs\":[{}]}}", last, items.join(",")))
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
        format!(
            "{{\"t\":\"lobby\",\"code\":\"{}\",\"members\":[{}],\"cpus\":[{}],\"players\":{},\"you\":{}}}",
            self.code,
            names.join(","),
            cpus.join(","),
            self.players(),
            you
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
        let players = self.players();
        // 席を配る。人が先に座り、残りが CPU。
        // 誰が 1 番手になるかは種で決めるので、部屋を立てた人が有利にはならない
        let seeds: [u32; 4] = [
            (now_ms() as u32) ^ 0x9e37_79b9,
            (now_ms() as u32).wrapping_mul(2654435761) ^ 0x1234_5678,
            (now_ms() as u32).wrapping_add(0x0f0f_0f0f),
            (now_ms() as u32).rotate_left(7) ^ 0xabcd_ef01,
        ];
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

        let mut msgs = Vec::new();
        for (mi, m) in self.members.iter().enumerate() {
            let seat = m.seat.unwrap_or(0);
            msgs.push((
                mi,
                format!(
                    "{{\"t\":\"start\",\"seeds\":[{},{},{},{}],\"players\":{},\
                     \"yourSeat\":{},\"names\":[{}],\"humans\":[{}],\"levels\":[{}]}}",
                    seeds[0], seeds[1], seeds[2], seeds[3], players, seat, names_json, humans_json,
                    levels_json
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
        let msg = format!("{{\"t\":\"act\",{echo},\"turn\":{turn},\"toAct\":{to_act}}}");
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
        let msg = format!("{{\"t\":\"act\",\"i\":{index},\"turn\":{turn},\"toAct\":{to_act}}}");
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
        let (over, turn, to_act) = (g.is_over(), g.turn, g.to_act);
        let msg = format!(
            "{{\"t\":\"act\",\"k\":\"maritime\",\"g\":[{},{},{},{},{}],\"w\":[{},{},{},{},{}],\"turn\":{turn},\"toAct\":{to_act}}}",
            give[0], give[1], give[2], give[3], give[4],
            take[0], take[1], take[2], take[3], take[4]
        );
        self.broadcast(&msg);
        if over {
            self.phase = Phase::Over;
            self.broadcast("{\"t\":\"over\"}");
        }
        self.bot_at = now_ms() + 550;
        Ok(())
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
