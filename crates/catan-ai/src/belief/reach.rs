//! 勝利ハザード（M3）: 「その人が次の自分の手番で 10 点に届く確率」を、
//! 粒子（手札・発展・山の仮説）ごとに計算する。
//!
//! 「公開 7 点 + 推定 VP 平均 1」のような平均値に潰さず、粒子ごとに
//! 「この手札・この札なら勝てるか」を通す（設計書 6.5）。
//!
//! 計算するもの: 次の手番の産出（出目 11 通りで周辺化）→ 銀行/港交換 → 建設・購入・
//! 発展カード 1 枚（騎士＝最大騎士力、街道建設＝最長路、収穫、独占）で届く点数。
//! 発展カードの購入は山の内訳（粒子の値）から超幾何で勝利点の確率を出す。
//!
//! 近似（明記）:
//! - 最長路は「本数」だけで判定する（道の形は見ない）→ 過大評価側
//! - 道を足すと新たに建てられる頂点が増える効果は見ない → 過小評価側
//! - 他人の手番の産出・7 の廃棄・盗みは見ない
//! - 交易（他プレイヤー）は見ない（自分が受けるかどうかは自分の判断なので、この関数の外で扱う）

use std::collections::HashMap;

use catan_core::action::{Bundle, DevCard, NUM_DEV_KINDS, EMPTY};
use catan_core::board::{BuildingKind, PlayerId, NUM_RESOURCES, RESOURCES};
use catan_core::game::{Game, COST_CITY, COST_DEV, COST_ROAD, COST_SETTLEMENT, MIN_ARMY, VP_TO_WIN};
use catan_core::topology::{EdgeId, NodeId, Topology, NUM_EDGES, NUM_NODES, NUM_TILES};

/// 発展カードの使い方の分岐
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum DevUse {
    None,
    Knight,
    RoadBuilding,
    YearOfPlenty,
    Monopoly(usize),
}

pub struct ReachSolver {
    pub p: PlayerId,
    rates: [u8; NUM_RESOURCES],
    bank: Bundle,
    /// 都市にできる開拓地の数（盤上の開拓地 と 残り駒 の小さい方）
    city_slots: u8,
    /// 今すぐ建てられる頂点の数 と 残り駒 の小さい方
    settle_slots: u8,
    roads_left: u8,
    can_road: bool,
    /// 最長路を取るのに要る道の本数（取れない/既に持つなら None）
    lr_needed: Option<u8>,
    /// 騎士 1 枚で最大騎士力を得られるか
    army_gain: bool,
    dev: [u8; NUM_DEV_KINDS],
    deck: [u8; NUM_DEV_KINDS],
    /// 独占で得られる資源別の枚数（他人の手札の合計。粒子の値）
    others: Bundle,
    /// 10 − 公開点 − 伏せ勝利点
    need_base: i32,
    /// 出目ごとの産出（2..=12）
    production: [Bundle; 13],
    memo: HashMap<[u8; 11], f32>,
    /// 現在の分岐の文脈
    ctx_army: bool,
    ctx_free_roads: u8,
    pub nodes: u64,
    /// 1 回の `win_prob_*` で展開してよい状態数。超えたら葉の値で打ち切る（下界側の近似）
    pub max_nodes: u64,
}

fn hyper_at_least(deck: &[u8; NUM_DEV_KINDS], draws: u8, need: i32) -> f32 {
    if need <= 0 {
        return 1.0;
    }
    let total: u32 = deck.iter().map(|&v| v as u32).sum();
    let vp = deck[DevCard::VictoryPoint.idx()] as u32;
    let d = (draws as u32).min(total);
    if d == 0 || vp == 0 || (need as u32) > d.min(vp) {
        return 0.0;
    }
    // P(X >= need), X ~ Hypergeom(total, vp, d)
    let comb = |n: u32, k: u32| -> f64 {
        if k > n {
            return 0.0;
        }
        let k = k.min(n - k);
        let mut r = 1.0f64;
        for i in 0..k {
            r = r * (n - i) as f64 / (i + 1) as f64;
        }
        r
    };
    let denom = comb(total, d);
    let mut acc = 0.0f64;
    for x in (need as u32)..=d.min(vp) {
        acc += comb(vp, x) * comb(total - vp, d - x);
    }
    (acc / denom) as f32
}

impl ReachSolver {
    /// `g` は公開局面（秘密は入っていなくてよい）。`dev`/`deck`/`others` は粒子の値。
    pub fn new(g: &Game, p: PlayerId, dev: [u8; NUM_DEV_KINDS], deck: [u8; NUM_DEV_KINDS], others: Bundle, bank: Bundle) -> Self {
        let topo = Topology::get();
        let q = p as usize;
        let ps = &g.players[q];
        let mut rates = [4u8; NUM_RESOURCES];
        for r in RESOURCES {
            rates[r.idx()] = g.board.best_maritime_rate(p, r);
        }
        let settlements_on_board = (0..NUM_NODES)
            .filter(|&n| matches!(g.board.building[n], Some(b) if b.owner == p && b.kind == BuildingKind::Settlement))
            .count() as u8;
        let city_slots = settlements_on_board.min(ps.cities_left);
        let buildable = (0..NUM_NODES).filter(|&n| g.can_place_settlement(n as NodeId, p, false)).count() as u8;
        let settle_slots = buildable.min(ps.settlements_left);
        let can_road = ps.roads_left > 0 && (0..NUM_EDGES).any(|e| g.can_build_road(e as EdgeId, p));
        let holder_len = g.longest_road_owner.map_or(0, |o| g.players[o as usize].longest_road);
        let lr_needed = if g.longest_road_owner == Some(p) || !can_road {
            None
        } else {
            let target = holder_len.max(4) + 1; // 5 以上、かつ現保持者より長く
            let need = target.saturating_sub(ps.longest_road);
            if need <= ps.roads_left { Some(need) } else { None }
        };
        let knights_after = ps.played_knights() + 1;
        let army_gain = dev[DevCard::Knight.idx()] > 0
            && knights_after >= MIN_ARMY
            && match g.largest_army_owner {
                None => true,
                Some(o) if o == p => false,
                Some(o) => knights_after > g.players[o as usize].played_knights(),
            };
        let hidden = dev[DevCard::VictoryPoint.idx()] as i32;
        let need_base = VP_TO_WIN as i32 - g.public_vp(p) as i32 - hidden;
        // 出目ごとの産出
        let mut production = [EMPTY; 13];
        for t in 0..NUM_TILES {
            if t as u8 == g.board.robber {
                continue;
            }
            let Some(res) = g.board.tile_resource[t] else { continue };
            let num = g.board.tile_number[t] as usize;
            if num == 0 {
                continue;
            }
            for n in topo.tile_nodes[t] {
                if let Some(b) = g.board.building[n as usize] {
                    if b.owner == p {
                        production[num][res.idx()] += match b.kind {
                            BuildingKind::Settlement => 1,
                            BuildingKind::City => 2,
                        };
                    }
                }
            }
        }
        ReachSolver {
            p,
            rates,
            bank,
            city_slots,
            settle_slots,
            roads_left: ps.roads_left,
            can_road,
            lr_needed,
            army_gain,
            dev,
            deck,
            others,
            need_base,
            production,
            memo: HashMap::new(),
            ctx_army: false,
            ctx_free_roads: 0,
            nodes: 0,
            max_nodes: 20_000,
        }
    }

    /// 既に届いているか（手番が回った瞬間に勝つ）
    pub fn already_won(&self) -> bool {
        self.need_base <= 0
    }

    /// 粗い上界で「どうやっても届かない」を弾く
    fn hopeless(&self, hand_total: u8) -> bool {
        let max_gain = self.city_slots as i32
            + self.settle_slots as i32
            + if self.lr_needed.is_some() { 2 } else { 0 }
            + if self.army_gain { 2 } else { 0 }
            + ((hand_total as i32 + 2) / 3).min(self.deck.iter().map(|&v| v as i32).sum()).min(self.deck[DevCard::VictoryPoint.idx()] as i32);
        self.need_base > max_gain
    }

    /// 出目を周辺化した「次の自分の手番で勝つ確率」
    pub fn win_prob_next_turn(&mut self, hand: &Bundle) -> f32 {
        if self.already_won() {
            return 1.0;
        }
        let mut acc = 0.0f32;
        for sum in 2..=12u8 {
            let ways = if sum == 7 { 6 } else { catan_core::board::pips(sum) };
            let p = ways as f32 / 36.0;
            let mut h = *hand;
            for r in 0..NUM_RESOURCES {
                h[r] += self.production[sum as usize][r];
            }
            acc += p * self.win_prob_now(&h);
        }
        acc
    }

    /// サイコロを振った後（産出込みの手札）から、この手番の残りで勝つ確率
    pub fn win_prob_now(&mut self, hand: &Bundle) -> f32 {
        if self.already_won() {
            return 1.0;
        }
        let total: u8 = hand.iter().sum();
        if self.hopeless(total.saturating_add(2)) {
            return 0.0;
        }
        let mut best = 0.0f32;
        let mut options = vec![DevUse::None];
        if self.dev[DevCard::Knight.idx()] > 0 && self.army_gain {
            options.push(DevUse::Knight);
        }
        if self.dev[DevCard::RoadBuilding.idx()] > 0 && self.can_road && self.lr_needed.is_some() {
            options.push(DevUse::RoadBuilding);
        }
        if self.dev[DevCard::YearOfPlenty.idx()] > 0 {
            options.push(DevUse::YearOfPlenty);
        }
        if self.dev[DevCard::Monopoly.idx()] > 0 {
            for r in 0..NUM_RESOURCES {
                if self.others[r] > 0 {
                    options.push(DevUse::Monopoly(r));
                }
            }
        }
        for opt in options {
            self.memo.clear();
            self.ctx_army = matches!(opt, DevUse::Knight);
            self.ctx_free_roads = if matches!(opt, DevUse::RoadBuilding) { 2u8.min(self.roads_left) } else { 0 };
            let mut h = *hand;
            let mut wild = 0u8;
            match opt {
                DevUse::YearOfPlenty => wild = 2,
                DevUse::Monopoly(r) => h[r] += self.others[r],
                _ => {}
            }
            let v = self.best(&h, wild, 0, 0, self.ctx_free_roads, 0);
            if v > best {
                best = v;
            }
            if best >= 1.0 - 1e-6 {
                break;
            }
        }
        best
    }

    fn need_now(&self, cities: u8, settles: u8, roads: u8) -> i32 {
        let mut gain = cities as i32 + settles as i32;
        if self.ctx_army {
            gain += 2;
        }
        if let Some(need) = self.lr_needed {
            if roads >= need {
                gain += 2;
            }
        }
        self.need_base - gain
    }

    fn try_pay(hand: &Bundle, wild: u8, cost: &Bundle) -> Option<(Bundle, u8)> {
        let mut h = *hand;
        let mut w = wild;
        for i in 0..NUM_RESOURCES {
            if h[i] >= cost[i] {
                h[i] -= cost[i];
            } else {
                let short = cost[i] - h[i];
                if w < short {
                    return None;
                }
                w -= short;
                h[i] = 0;
            }
        }
        Some((h, w))
    }

    fn best(&mut self, hand: &Bundle, wild: u8, cities: u8, settles: u8, roads: u8, devs: u8) -> f32 {
        self.nodes += 1;
        let need = self.need_now(cities, settles, roads);
        if need <= 0 {
            return 1.0;
        }
        let mut v = hyper_at_least(&self.deck, devs, need);
        if v >= 1.0 - 1e-6 {
            return v;
        }
        // 残りの資源で届き得る上界（1 点に最低 3 枚）。届かなければ展開しない
        let cards = hand.iter().map(|&c| c as i32).sum::<i32>() + wild as i32;
        let mut bound = (self.city_slots - cities) as i32 + (self.settle_slots - settles) as i32;
        if let Some(nr) = self.lr_needed {
            if roads < nr {
                bound += 2;
            }
        }
        bound = bound.min((cards + 2) / 3 + 2) + self.deck[DevCard::VictoryPoint.idx()].min(((cards + 2) / 3) as u8) as i32;
        if need > bound || self.nodes > self.max_nodes {
            return v;
        }
        let key = [hand[0], hand[1], hand[2], hand[3], hand[4], wild, cities, settles, roads, devs, 0];
        if let Some(&m) = self.memo.get(&key) {
            return m;
        }
        let deck_total: u8 = self.deck.iter().sum();
        // 都市
        if cities < self.city_slots {
            if let Some((h, w)) = Self::try_pay(hand, wild, &COST_CITY) {
                v = v.max(self.best(&h, w, cities + 1, settles, roads, devs));
            }
        }
        // 開拓地
        if v < 1.0 && settles < self.settle_slots {
            if let Some((h, w)) = Self::try_pay(hand, wild, &COST_SETTLEMENT) {
                v = v.max(self.best(&h, w, cities, settles + 1, roads, devs));
            }
        }
        // 道（最長路が狙える時だけ）
        if v < 1.0 {
            if let Some(need_roads) = self.lr_needed {
                if roads < need_roads && roads < self.roads_left {
                    if let Some((h, w)) = Self::try_pay(hand, wild, &COST_ROAD) {
                        v = v.max(self.best(&h, w, cities, settles, roads + 1, devs));
                    }
                }
            }
        }
        // 発展カード購入
        if v < 1.0 && devs < deck_total {
            if let Some((h, w)) = Self::try_pay(hand, wild, &COST_DEV) {
                v = v.max(self.best(&h, w, cities, settles, roads, devs + 1));
            }
        }
        // 銀行・港との交換
        if v < 1.0 {
            for r in 0..NUM_RESOURCES {
                let rate = self.rates[r];
                if hand[r] < rate {
                    continue;
                }
                for s in 0..NUM_RESOURCES {
                    if s == r || self.bank[s] == 0 {
                        continue;
                    }
                    let mut h = *hand;
                    h[r] -= rate;
                    h[s] += 1;
                    v = v.max(self.best(&h, wild, cities, settles, roads, devs));
                    if v >= 1.0 {
                        break;
                    }
                }
                if v >= 1.0 {
                    break;
                }
            }
        }
        self.memo.insert(key, v);
        v
    }
}
