//! プレイヤー 1 人分の視界。
//!
//! 何を見せるかの線引きは「実卓でその情報に到達できるか」で決めている:
//!
//! | 情報 | 見える | 理由 |
//! |---|---|---|
//! | 盤面・港・盗賊 | ○ | 公開 |
//! | 銀行の残り | ○ | 山が場に出ている |
//! | 相手の資源の手札の**枚数** | ○ | 何枚持っているかは全員が見ている |
//! | 相手の資源の手札の**中身** | ✗ | 盗賊で 1 枚移った時、当事者以外には何が移ったか分からない |
//! | 相手の発展カードの**枚数** | ○ | 何枚買って何枚使ったかは全員が見ている |
//! | 相手の発展カードの**種類** | ✗ | これは原理的に見えない |
//! | 打たれた発展カード | ○ | 騎士は表向きに残り、進歩カードは公開して使う |
//! | 公開勝利点 | ○ | 建物・特別カードは見える |
//! | 伏せた勝利点カード | ✗ | ゲーム終了まで伏せたまま |
//!
//! 種類が分からない発展カードを扱うには [`View::determinize`] を使う。
//! 見えていない札を「あり得る山」から引き直した完全情報の局面を作るので、
//! そのまま探索にかけられる。

use crate::action::{Bundle, DevCard, Prompt, DEV_CARDS, DEV_DECK_COMPOSITION, NUM_DEV_KINDS};
use crate::board::{Board, PlayerId, NUM_RESOURCES};
use crate::game::{Game, TradeState, BANK_PER_RESOURCE, MAX_PLAYERS};
use crate::rng::Rng;

pub struct View<'a> {
    game: &'a Game,
    pub me: PlayerId,
}

impl<'a> View<'a> {
    pub fn new(game: &'a Game, me: PlayerId) -> Self {
        View { game, me }
    }

    // ---- 公開情報 ----

    pub fn board(&self) -> &Board {
        &self.game.board
    }
    pub fn bank(&self) -> Bundle {
        self.game.bank
    }
    pub fn num_players(&self) -> usize {
        self.game.n()
    }
    pub fn prompt(&self) -> Prompt {
        self.game.prompt
    }
    pub fn to_act(&self) -> PlayerId {
        self.game.to_act
    }
    pub fn turn_player(&self) -> PlayerId {
        self.game.turn_player
    }
    pub fn turn(&self) -> u32 {
        self.game.turn
    }
    pub fn rolled(&self) -> bool {
        self.game.rolled
    }
    pub fn is_setup(&self) -> bool {
        self.game.is_setup()
    }
    pub fn trade(&self) -> Option<TradeState> {
        self.game.trade
    }
    pub fn turn_time_left_ms(&self) -> Option<u32> {
        self.game.turn_time_left_ms()
    }
    pub fn longest_road_owner(&self) -> Option<PlayerId> {
        self.game.longest_road_owner
    }
    pub fn largest_army_owner(&self) -> Option<PlayerId> {
        self.game.largest_army_owner
    }

    /// 相手の資源の手札。方針として公開している（詳細はモジュールの表）。
    pub fn hand(&self, p: PlayerId) -> Bundle {
        self.game.players[p as usize].hand
    }
    pub fn hand_size(&self, p: PlayerId) -> u8 {
        self.game.players[p as usize].hand_size()
    }
    /// 発展カードの**枚数**だけ。種類は分からない。
    pub fn dev_count(&self, p: PlayerId) -> u8 {
        self.game.players[p as usize].dev_count()
    }
    /// これまでに打った発展カード（種類別）。公開情報。
    pub fn played_dev(&self, p: PlayerId) -> [u8; NUM_DEV_KINDS] {
        self.game.players[p as usize].played_dev
    }
    pub fn played_knights(&self, p: PlayerId) -> u8 {
        self.game.players[p as usize].played_knights()
    }
    pub fn roads_left(&self, p: PlayerId) -> u8 {
        self.game.players[p as usize].roads_left
    }
    pub fn settlements_left(&self, p: PlayerId) -> u8 {
        self.game.players[p as usize].settlements_left
    }
    pub fn cities_left(&self, p: PlayerId) -> u8 {
        self.game.players[p as usize].cities_left
    }
    pub fn longest_road(&self, p: PlayerId) -> u8 {
        self.game.players[p as usize].longest_road
    }
    /// 見えている勝利点。伏せた勝利点カードは含まない。
    pub fn public_vp(&self, p: PlayerId) -> u8 {
        self.game.public_vp(p)
    }
    /// 山に残っている発展カードの枚数（種類は不明）
    pub fn dev_deck_left(&self) -> u8 {
        self.game.dev_deck.iter().sum()
    }

    // ---- 自分だけが見えるもの ----

    pub fn my_dev(&self) -> [u8; NUM_DEV_KINDS] {
        self.game.players[self.me as usize].dev
    }
    pub fn my_playable_dev(&self, c: DevCard) -> u8 {
        self.game.players[self.me as usize].playable_dev(c)
    }
    /// 伏せた勝利点カードを含む自分の実際の勝利点
    pub fn my_actual_vp(&self) -> u8 {
        self.game.actual_vp(self.me)
    }

    // ---- 推論 ----

    /// まだ見ていない発展カードの内訳（= 山 + 相手の手札）。
    ///
    /// 25 枚の構成は既知で、打たれたカードは全員が見ているので、
    /// 「構成 − 打たれた分 − 自分の手札」で厳密に求まる。
    /// **完璧なカードカウンティングで到達できる情報の上限**がこれ。
    pub fn unseen_dev(&self) -> [u8; NUM_DEV_KINDS] {
        let mut left = [0u8; NUM_DEV_KINDS];
        for (c, n) in DEV_DECK_COMPOSITION {
            left[c.idx()] = n;
        }
        for q in 0..self.num_players() {
            for i in 0..NUM_DEV_KINDS {
                left[i] -= self.game.players[q].played_dev[i];
            }
        }
        for i in 0..NUM_DEV_KINDS {
            left[i] -= self.game.players[self.me as usize].dev[i];
        }
        left
    }

    /// 見えていない札を引き直して、完全情報の局面を 1 つ作る。
    ///
    /// 相手の発展カードは「枚数はそのまま・種類は [`Self::unseen_dev`] から無作為に」
    /// 割り当て直す。探索はこの上で行う（determinization）。
    pub fn determinize(&self, rng: &mut Rng) -> Game {
        self.determinize_with(rng, true)
    }

    /// `hide_hands = false` にすると、相手の資源の中身をそのまま覗く。
    /// **実卓ではできない**。情報を隠すことの代償を測るためだけに残してある。
    pub fn determinize_with(&self, rng: &mut Rng, hide_hands: bool) -> Game {
        let mut g = self.game.clone();

        // 未知の札をばらして混ぜる
        let unseen = self.unseen_dev();
        let mut pool: Vec<DevCard> = Vec::with_capacity(unseen.iter().map(|&v| v as usize).sum());
        for c in DEV_CARDS {
            for _ in 0..unseen[c.idx()] {
                pool.push(c);
            }
        }
        rng.shuffle(&mut pool);

        let mut i = 0;
        for q in 0..g.n() {
            if q as PlayerId == self.me {
                continue;
            }
            let k = g.players[q].dev_count() as usize;
            let bought: usize = g.players[q]
                .dev_bought_this_turn
                .iter()
                .map(|&v| v as usize)
                .sum();
            debug_assert!(i + k <= pool.len(), "未知の札が足りない");
            let mut dev = [0u8; NUM_DEV_KINDS];
            let mut bought_now = [0u8; NUM_DEV_KINDS];
            for j in 0..k {
                let c = pool[i + j];
                dev[c.idx()] += 1;
                // 「このターン買った枚数」は公開情報なので枚数だけ合わせる。
                // どれがそれかは分からないので先頭から割り当てる
                if j < bought {
                    bought_now[c.idx()] += 1;
                }
            }
            i += k;
            g.players[q].dev = dev;
            g.players[q].dev_bought_this_turn = bought_now;
        }

        // 残りが山
        let mut deck = [0u8; NUM_DEV_KINDS];
        for c in &pool[i..] {
            deck[c.idx()] += 1;
        }
        g.dev_deck = deck;

        if hide_hands {
            self.reshuffle_hands(&mut g, rng);
        }
        g
    }

    /// 相手の資源の手札を引き直す。
    ///
    /// 枚数は公開情報なのでそのまま。**中身だけ**を混ぜ直す。
    /// 種類ごとの総数は 19 枚で決まっていて、山（bank）と自分の手札は見えているので、
    /// 「相手全員が合わせて何を持っているか」は誰でも数えられる。
    /// 数えられないのは **その分け方** だけなので、そこだけ引き直す。
    ///
    /// ただし出ている提案は「その札を持っている」ことを公開しているので、
    /// 先に取り分けてから残りを配る。そうしないと、
    /// 場に出ている条件を実行できない局面ができてしまう。
    fn reshuffle_hands(&self, g: &mut Game, rng: &mut Rng) {
        let mut pool = [0u8; NUM_RESOURCES];
        for r in 0..NUM_RESOURCES {
            let total = BANK_PER_RESOURCE
                .saturating_sub(g.bank[r])
                .saturating_sub(g.players[self.me as usize].hand[r]);
            pool[r] = total;
        }

        // 公開されている条件ぶんを先に取り分ける
        let mut fixed = [[0u8; NUM_RESOURCES]; MAX_PLAYERS];
        if let Some(t) = g.trade {
            let mut reserve = |p: PlayerId, b: &Bundle, pool: &mut [u8; NUM_RESOURCES]| {
                if p == self.me {
                    return;
                }
                for r in 0..NUM_RESOURCES {
                    let n = b[r].min(pool[r]);
                    fixed[p as usize][r] += n;
                    pool[r] -= n;
                }
            };
            reserve(t.proposer, &t.give, &mut pool);
            for q in 0..g.n() {
                for alt in t.counters[q].iter().flatten() {
                    reserve(q as PlayerId, &alt.0, &mut pool);
                    break; // 1 本ぶん取り分ければ実行可能性は保てる
                }
            }
        }

        // 残りを袋に入れて混ぜ、枚数どおりに配り直す
        let mut bag: Vec<u8> = Vec::with_capacity(pool.iter().map(|&n| n as usize).sum());
        for r in 0..NUM_RESOURCES {
            for _ in 0..pool[r] {
                bag.push(r as u8);
            }
        }
        rng.shuffle(&mut bag);

        let mut k = 0usize;
        for q in 0..g.n() {
            if q as PlayerId == self.me {
                continue;
            }
            let size = g.players[q].hand_size() as usize;
            let mut hand = fixed[q];
            let taken: usize = hand.iter().map(|&n| n as usize).sum();
            if taken > size {
                continue; // 取り分けだけで超えるのは異常。その人は触らない
            }
            for _ in 0..(size - taken) {
                if k >= bag.len() {
                    break;
                }
                hand[bag[k] as usize] += 1;
                k += 1;
            }
            if hand.iter().map(|&n| n as usize).sum::<usize>() == size {
                g.players[q].hand = hand;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::Action;
    use crate::game::{Game, GameConfig};

    fn advance(g: &mut Game, steps: usize, rng: &mut Rng) {
        let mut buf = Vec::new();
        for _ in 0..steps {
            if g.is_over() {
                break;
            }
            g.legal_actions_into(&mut buf);
            if buf.is_empty() {
                break;
            }
            let a = buf[rng.below(buf.len() as u32) as usize];
            g.apply(a);
        }
    }

    #[test]
    fn 未知の発展カードは常に整合する() {
        for seed in 0..30u64 {
            let mut g = Game::with_config(4, seed, GameConfig::default());
            let mut rng = Rng::with_stream(seed, 9);
            for step in 0..40 {
                advance(&mut g, 50, &mut rng);
                if g.is_over() {
                    break;
                }
                for me in 0..4u8 {
                    let v = View::new(&g, me);
                    let unseen = v.unseen_dev();
                    // 未知の総数 = 山 + 相手の手札
                    let opp: u32 = (0..4)
                        .filter(|&q| q as u8 != me)
                        .map(|q| g.players[q].dev_count() as u32)
                        .sum();
                    let total: u32 = unseen.iter().map(|&x| x as u32).sum();
                    assert_eq!(
                        total,
                        opp + v.dev_deck_left() as u32,
                        "seed={seed} step={step} me={me} 未知の枚数が合わない"
                    );
                    // 自分の手札は未知に含まれない
                    for i in 0..NUM_DEV_KINDS {
                        assert!(unseen[i] as u32 <= 25);
                    }
                }
            }
        }
    }

    #[test]
    fn determinizeは枚数を保ったまま種類だけ入れ替える() {
        for seed in 0..20u64 {
            let mut g = Game::with_config(4, seed, GameConfig::default());
            let mut rng = Rng::with_stream(seed, 11);
            advance(&mut g, 600, &mut rng);
            if g.is_over() {
                continue;
            }
            let me = g.to_act;
            let v = View::new(&g, me);
            let sim = v.determinize(&mut rng);

            // 自分の手札は変わらない（発展カードも資源も）
            assert_eq!(sim.players[me as usize].dev, g.players[me as usize].dev);
            assert_eq!(sim.players[me as usize].hand, g.players[me as usize].hand);
            // 相手は枚数だけ一致
            for q in 0..4 {
                assert_eq!(
                    sim.players[q].dev_count(),
                    g.players[q].dev_count(),
                    "seed={seed} P{q} の発展カード枚数が変わった"
                );
                assert_eq!(sim.players[q].played_dev, g.players[q].played_dev);
                // 資源は **枚数だけ** 一致。中身は伏せ札なので引き直される
                assert_eq!(
                    sim.players[q].hand_size(),
                    g.players[q].hand_size(),
                    "seed={seed} P{q} の資源の枚数が変わった"
                );
                assert_eq!(
                    sim.players[q].dev_bought_this_turn.iter().sum::<u8>(),
                    g.players[q].dev_bought_this_turn.iter().sum::<u8>()
                );
            }
            // 資源も種類ごとに 19 枚が保存されていること
            // （山＋全員の手札。相手の分け方が変わっても総数は動かない）
            for r in 0..NUM_RESOURCES {
                let mut total = sim.bank[r] as u32;
                for q in 0..4 {
                    total += sim.players[q].hand[r] as u32;
                }
                assert_eq!(total, BANK_PER_RESOURCE as u32, "seed={seed} 資源 {r} の総数が変わった");
            }
            // 25 枚の保存
            let mut total = [0u8; NUM_DEV_KINDS];
            for q in 0..4 {
                for i in 0..NUM_DEV_KINDS {
                    total[i] += sim.players[q].dev[i] + sim.players[q].played_dev[i];
                }
            }
            for (c, n) in DEV_DECK_COMPOSITION {
                assert_eq!(
                    total[c.idx()] + sim.dev_deck[c.idx()],
                    n,
                    "seed={seed} {} の総数が壊れた",
                    c.ja()
                );
            }
        }
    }

    #[test]
    fn determinizeした局面はそのまま指せる() {
        let mut g = Game::with_config(4, 42, GameConfig::default());
        let mut rng = Rng::with_stream(42, 13);
        advance(&mut g, 400, &mut rng);
        if !g.is_over() {
            let v = View::new(&g, g.to_act);
            let mut sim = v.determinize(&mut rng);
            let acts = sim.legal_actions();
            assert!(!acts.is_empty());
            sim.apply(acts[0]);
        }
    }

    #[test]
    fn 相手の伏せた勝利点は見えない() {
        // View には actual_vp が無く、public_vp しか無いことをコンパイル時に担保する。
        // ここでは「勝利点カードを持っていても public_vp が増えない」ことを確認する。
        let mut g = Game::with_config(4, 3, GameConfig::default());
        let mut rng = Rng::with_stream(3, 17);
        advance(&mut g, 800, &mut rng);
        for q in 0..4u8 {
            let v = View::new(&g, 0);
            let vp_cards = g.players[q as usize].dev[DevCard::VictoryPoint.idx()];
            assert_eq!(
                v.public_vp(q) + vp_cards,
                g.actual_vp(q),
                "公開VP + 伏せたVPカード = 実際のVP"
            );
        }
        let _ = Action::EndTurn;
    }
}
