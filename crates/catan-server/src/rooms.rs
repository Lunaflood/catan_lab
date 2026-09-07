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
use std::net::TcpStream;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub const MAX_PLAYERS: usize = 4;

/// 参加者ひとり。CPU は `token` を持たない
pub struct Member {
    pub token: String,
    pub name: String,
    /// 開いている配信の口。切れたら落とす
    pub feeds: Vec<TcpStream>,
    pub seat: Option<usize>,
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

    pub fn broadcast(&mut self, msg: &str) {
        for m in &mut self.members {
            m.feeds.retain_mut(|f| crate::http::push(f, msg));
        }
    }

    pub fn send_to(&mut self, mi: usize, msg: &str) {
        if let Some(m) = self.members.get_mut(mi) {
            m.feeds.retain_mut(|f| crate::http::push(f, msg));
        }
    }

    /// 待機所の様子。席は開始時に決まるので、ここでは並びだけ知らせる
    pub fn lobby_json(&self) -> String {
        let names: Vec<String> = self
            .members
            .iter()
            .map(|m| format!("{{\"name\":\"{}\"}}", crate::http::esc(&m.name)))
            .collect();
        let cpus: Vec<String> = self.cpus.iter().map(|l| l.to_string()).collect();
        format!(
            "{{\"t\":\"lobby\",\"code\":\"{}\",\"members\":[{}],\"cpus\":[{}],\"players\":{}}}",
            self.code,
            names.join(","),
            cpus.join(","),
            self.players()
        )
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
        self.bots = (0..players)
            .map(|seat| {
                if self.seat_member[seat].is_some() {
                    None
                } else {
                    let lv = *self.cpus.get(ci).unwrap_or(&1);
                    ci += 1;
                    Some(bot_for_level(lv, seeds[1] as u64 * 31 + seat as u64))
                }
            })
            .collect();

        self.seeds = seeds;
        self.game = Some(game);
        self.phase = Phase::Playing;
        self.again.clear();
        self.bot_at = now_ms() + 700;

        // 席ごとに「自分の席」が違うので、1 人ずつ別の内容を送る
        let names: Vec<String> = (0..players)
            .map(|seat| match self.seat_member[seat] {
                Some(mi) => crate::http::esc(&self.members[mi].name),
                None => {
                    let lv = self.bots[seat].as_ref().map(|b| b.name()).unwrap_or_default();
                    crate::http::esc(&lv)
                }
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

        let mut msgs = Vec::new();
        for (mi, m) in self.members.iter().enumerate() {
            let seat = m.seat.unwrap_or(0);
            msgs.push((
                mi,
                format!(
                    "{{\"t\":\"start\",\"seeds\":[{},{},{},{}],\"players\":{},\
                     \"yourSeat\":{},\"names\":[{}],\"humans\":[{}]}}",
                    seeds[0], seeds[1], seeds[2], seeds[3], players, seat, names_json, humans_json
                ),
            ));
        }
        for (mi, msg) in msgs {
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
        g.apply(a);
        let over = g.is_over();
        let turn = g.turn;
        let to_act = g.to_act;
        let msg = format!("{{\"t\":\"act\",{echo},\"turn\":{turn},\"toAct\":{to_act}}}");
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
        g.apply(a);
        let over = g.is_over();
        let turn = g.turn;
        let to_act = g.to_act;
        self.broadcast(&format!(
            "{{\"t\":\"act\",\"i\":{index},\"turn\":{turn},\"toAct\":{to_act}}}"
        ));
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
                    g.apply(Action::MaritimeTrade { give: *r, count: rate, take: queue[qi] });
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
        let l = self.lobby_json();
        self.broadcast(&l);
    }
}
