//! 自動対戦ハーネス。
//!
//! 席の割り当ては毎試合**全順列を巡回**する。単純なローテーションだと
//! 各ボットが全部の席に座る一方で「誰の次に座るか」が固定されてしまい、
//! 強いボットの直後に座る役だけが不利になる（実測で 11.8% vs 17.9% の差が出た）。
//! 順列を回せば相対順序も均される。

use crate::bots::Bot;
use catan_core::action::Action;
use catan_core::board::PlayerId;
use catan_core::game::{Game, GameConfig, MAX_PLAYERS};
use catan_core::view::View;

/// 1 試合あたりの行動数の上限。無限ループの保険。
pub const MAX_ACTIONS_PER_GAME: usize = 100_000;
/// 1 試合あたりの手番数の上限。
///
/// カタンには**本物の膠着**がある（全員が開拓地を都市に変え切り、拡張先が無く、
/// 発展カードの山も尽きると誰も 10 点に届かない）。実測で 19,498 手番の対局が出た。
/// 手番で切っておかないと、膠着した 1 局が測定時間の大半を食う。
pub const MAX_TURNS_PER_GAME: u32 = 400;

#[derive(Clone, Debug)]
pub struct MatchResult {
    pub names: Vec<String>,
    pub games: usize,
    /// 決着した試合数（上限に当たった試合は含まない）
    pub finished: usize,
    /// 膠着して打ち切った試合数
    pub stalled: usize,
    pub wins: Vec<usize>,
    pub total_vp: Vec<u64>,
    pub total_turns: u64,
    pub total_actions: u64,
    pub elapsed_secs: f64,
    /// 交渉の内訳。Guhe & Lascarides 2014 は「提案数・成立数・総交易数」を見ることで
    /// どの変更が効いたかを切り分けている。同じ計器を最初から持っておく。
    pub offers: u64,
    pub counters: u64,
    pub confirmed: u64,
    pub counters_accepted: u64,
}

impl MatchResult {
    pub fn win_rate(&self, i: usize) -> f64 {
        self.wins[i] as f64 / self.finished.max(1) as f64
    }
    /// 勝率の標準誤差（二項分布）
    pub fn win_rate_stderr(&self, i: usize) -> f64 {
        let p = self.win_rate(i);
        (p * (1.0 - p) / self.finished.max(1) as f64).sqrt()
    }
    pub fn avg_vp(&self, i: usize) -> f64 {
        self.total_vp[i] as f64 / self.finished.max(1) as f64
    }
    pub fn avg_turns(&self) -> f64 {
        self.total_turns as f64 / self.finished.max(1) as f64
    }
    pub fn games_per_sec(&self) -> f64 {
        self.games as f64 / self.elapsed_secs.max(1e-9)
    }

    pub fn report(&self) -> String {
        let n = self.names.len();
        let null = 1.0 / n as f64;
        let mut s = String::new();
        s.push_str(&format!(
            "{} 戦 (決着 {}) / 平均 {:.1} 手番 / {:.0} ゲーム毎秒\n",
            self.games,
            self.finished,
            self.avg_turns(),
            self.games_per_sec()
        ));
        s.push_str(&format!(
            "帰無仮説の勝率 = {:.1}%（{} 人）\n",
            null * 100.0,
            n
        ));
        for i in 0..n {
            let p = self.win_rate(i);
            let se = self.win_rate_stderr(i);
            let z = if se > 0.0 { (p - null) / se } else { 0.0 };
            s.push_str(&format!(
                "  {:<16} 勝率 {:>5.1}% ± {:.1}  平均VP {:.2}  (z = {:+.1}{})\n",
                self.names[i],
                p * 100.0,
                se * 100.0,
                self.avg_vp(i),
                z,
                if z.abs() > 2.58 { ", p<0.01" } else { "" }
            ));
        }
        let f = self.finished.max(1) as f64;
        s.push_str(&format!(
            "交渉: 1試合あたり 提案 {:.1} / 対案 {:.1} / 成立 {:.1}（うち対案での成立 {:.1}）
",
            self.offers as f64 / f,
            self.counters as f64 / f,
            (self.confirmed + self.counters_accepted) as f64 / f,
            self.counters_accepted as f64 / f
        ));
        s
    }
}

/// 0..n の全順列を辞書順で返す。
fn permutations(n: usize) -> Vec<Vec<usize>> {
    let mut cur: Vec<usize> = (0..n).collect();
    let mut out = vec![cur.clone()];
    loop {
        // 次の順列（標準的な next_permutation）
        let Some(i) = (0..n.saturating_sub(1)).rev().find(|&i| cur[i] < cur[i + 1]) else {
            break;
        };
        let j = (i + 1..n).rev().find(|&j| cur[j] > cur[i]).unwrap();
        cur.swap(i, j);
        cur[i + 1..].reverse();
        out.push(cur.clone());
    }
    out
}

/// `bots` を戦わせる。席の割り当ては全順列を巡回する。
pub fn run_match(
    bots: &mut [Box<dyn Bot>],
    games: usize,
    seed_base: u64,
    cfg: GameConfig,
) -> MatchResult {
    let n = bots.len();
    assert!((3..=MAX_PLAYERS).contains(&n), "3〜4 体で回す");

    let names: Vec<String> = bots.iter().map(|b| b.name()).collect();
    let mut wins = vec![0usize; n];
    let mut total_vp = vec![0u64; n];
    let mut finished = 0usize;
    let mut stalled = 0usize;
    let mut total_turns = 0u64;
    let mut total_actions = 0u64;
    let (mut offers, mut counters, mut confirmed, mut counters_accepted) = (0u64, 0u64, 0u64, 0u64);

    let t0 = std::time::Instant::now();
    let mut buf: Vec<Action> = Vec::with_capacity(64);

    let perms = permutations(n);
    for gi in 0..games {
        // bot_at_seat[s] = 席 s に座るボットの番号
        let bot_at_seat = &perms[gi % perms.len()];

        let seed = seed_base.wrapping_add(gi as u64);
        for (b, bot) in bots.iter_mut().enumerate() {
            bot.reset(seed ^ ((b as u64 + 1) << 32));
        }

        let mut g = Game::with_config(n as u8, seed, cfg);
        let mut actions = 0usize;
        while !g.is_over() && actions < MAX_ACTIONS_PER_GAME && g.turn < MAX_TURNS_PER_GAME {
            g.legal_actions_into(&mut buf);
            debug_assert!(!buf.is_empty());
            let seat = g.to_act as usize;
            let b = bot_at_seat[seat];
            let view = View::new(&g, seat as PlayerId);
            let a = bots[b].decide(&view, &buf);
            match a {
                Action::OfferTrade { .. } => offers += 1,
                Action::CounterOffer { .. } => counters += 1,
                Action::ConfirmTrade(_) => confirmed += 1,
                Action::AcceptCounter { .. } => counters_accepted += 1,
                _ => {}
            }
            g.apply(a);
            actions += 1;
        }
        total_actions += actions as u64;

        if g.winner.is_none() {
            stalled += 1;
        }
        if let Some(w) = g.winner {
            finished += 1;
            total_turns += g.turn as u64;
            wins[bot_at_seat[w as usize]] += 1;
            for s in 0..n {
                total_vp[bot_at_seat[s]] += g.actual_vp(s as PlayerId) as u64;
            }
        }
    }

    MatchResult {
        names,
        games,
        finished,
        stalled,
        wins,
        total_vp,
        total_turns,
        total_actions,
        elapsed_secs: t0.elapsed().as_secs_f64(),
        offers,
        counters,
        confirmed,
        counters_accepted,
    }
}
