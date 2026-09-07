//! 観測履歴の公開台帳（M1/M2）。
//!
//! 推定（Belief）が全滅した時に、**観測だけから**制約を組み直すための材料。
//! 内部カード ID は持たない。持つのは「いつ・何枚買ったか」「いつ・何を使ったか」だけ。
//! 自分の札の種類は本人の私的観測として別に持つ。

use catan_core::action::{Bundle, DevCard, NUM_DEV_KINDS, EMPTY};
use catan_core::board::{PlayerId, NUM_RESOURCES};
use catan_core::game::MAX_PLAYERS;
use catan_core::observation::{ObservedEvent, VisibleEvent};

use crate::opponent::Opportunity;

/// 発展カードの購入 1 件（同じ手番の複数購入は 1 件にまとめる）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Purchase {
    pub turn: u32,
    pub count: u8,
    /// 購入時点で本人が終えていた手番の数（保持した「自分の手番数」を数えるため）
    pub turns_ended_before: u32,
}

/// 1 人分の公開台帳
#[derive(Clone, Debug, Default)]
pub struct PlayerLedger {
    /// 発展カードの購入。内部カード ID は持たない
    pub purchases: Vec<Purchase>,
    /// 発展カードの使用（手番番号, 種類）
    pub plays: Vec<(u32, DevCard)>,
    /// 使用済み（種類別）。`plays` の集計
    pub played_dev: [u8; NUM_DEV_KINDS],
    /// 本人が終えた手番の数
    pub turns_ended: u32,
}

/// 「使用可能な自分の手番を終えた」1 回分の記録（校正・説明用）
#[derive(Clone, Copy, Debug)]
pub struct TurnRecord {
    pub player: PlayerId,
    pub turn: u32,
    /// 手番終了時点で持っていた発展カードの枚数（公開）
    pub dev_count: u8,
    /// うち、その手番に買った枚数（使えない）
    pub bought_this_turn: u8,
    /// この手番に何かを使ったか
    pub played: Option<DevCard>,
    pub opportunity: Opportunity,
}

/// 開いている交易の公開条件（推定を組み直す時の資源の下限）
#[derive(Clone, Copy, Debug, Default)]
pub struct OpenTrade {
    pub proposer: PlayerId,
    pub give: Bundle,
    pub want: Bundle,
    pub accepted: [bool; MAX_PLAYERS],
    /// 各人が出した対案・候補の give（資源ごとの最大値で持つ）
    pub counter_give_max: [Bundle; MAX_PLAYERS],
}

#[derive(Clone, Debug)]
pub struct History {
    pub viewer: PlayerId,
    pub n: usize,
    pub ledgers: [PlayerLedger; MAX_PLAYERS],
    /// 自分が買った札（手番番号, 種類）。本人だけの私的観測
    pub own_purchases: Vec<(u32, DevCard)>,
    pub open_trade: Option<OpenTrade>,
    pub turns: Vec<TurnRecord>,
    /// 直近の手番番号と手番プレイヤー
    pub turn: u32,
    pub turn_player: PlayerId,
    pub events: u64,
    pub last_seq: u64,
    /// 手番の途中で使われた札（EndTurn で `TurnRecord` に写す）
    played_this_turn: Option<DevCard>,
    /// 手番の開始時点の使用機会（盗賊で塞がれていた等は手番の途中で変わるため）
    turn_start_opportunity: [Option<Opportunity>; MAX_PLAYERS],
}

impl History {
    pub fn new(viewer: PlayerId, n: usize) -> Self {
        History {
            viewer,
            n,
            ledgers: Default::default(),
            own_purchases: Vec::new(),
            open_trade: None,
            turns: Vec::new(),
            turn: 0,
            turn_player: 0,
            events: 0,
            last_seq: 0,
            played_this_turn: None,
            turn_start_opportunity: [None; MAX_PLAYERS],
        }
    }

    /// 出来事を台帳に写す。順序番号が戻った出来事は無視する（重複配信の防止）。
    /// 戻り値は「新しい出来事として受理したか」。
    pub fn record(&mut self, ev: &ObservedEvent) -> bool {
        if ev.seq != 0 && ev.seq <= self.last_seq {
            return false;
        }
        self.last_seq = ev.seq;
        self.events += 1;
        self.turn = ev.turn;
        self.turn_player = ev.turn_player;
        let a = ev.actor as usize;
        let opp = Opportunity::from_context(&ev.before, ev.actor, self.n);
        if self.turn_start_opportunity[a].is_none() && ev.actor == ev.turn_player {
            self.turn_start_opportunity[a] = Some(opp);
        }
        match ev.payload {
            VisibleEvent::BuyDev { card } => {
                let l = &mut self.ledgers[a];
                match l.purchases.last_mut() {
                    Some(p) if p.turn == ev.turn => p.count += 1,
                    _ => {
                        let before = l.turns_ended;
                        l.purchases.push(Purchase { turn: ev.turn, count: 1, turns_ended_before: before })
                    }
                }
                if let Some(c) = card {
                    debug_assert_eq!(ev.actor, self.viewer, "他人の札の種類が届いた");
                    self.own_purchases.push((ev.turn, c));
                }
            }
            VisibleEvent::PlayKnight => self.note_play(a, ev.turn, DevCard::Knight),
            VisibleEvent::PlayRoadBuilding => self.note_play(a, ev.turn, DevCard::RoadBuilding),
            VisibleEvent::PlayYearOfPlenty { .. } => self.note_play(a, ev.turn, DevCard::YearOfPlenty),
            VisibleEvent::PlayMonopoly { .. } => self.note_play(a, ev.turn, DevCard::Monopoly),
            VisibleEvent::Offer { give, want } => {
                self.open_trade = Some(OpenTrade {
                    proposer: ev.actor,
                    give,
                    want,
                    accepted: [false; MAX_PLAYERS],
                    counter_give_max: [EMPTY; MAX_PLAYERS],
                });
            }
            VisibleEvent::Accept => {
                if let Some(t) = self.open_trade.as_mut() {
                    t.accepted[a] = true;
                }
            }
            VisibleEvent::Counter { give, .. } | VisibleEvent::CounterAlt { give, .. } => {
                if let Some(t) = self.open_trade.as_mut() {
                    for r in 0..NUM_RESOURCES {
                        t.counter_give_max[a][r] = t.counter_give_max[a][r].max(give[r]);
                    }
                }
            }
            VisibleEvent::Confirm { .. } | VisibleEvent::AcceptCounter { .. } | VisibleEvent::Cancel => {
                self.open_trade = None;
            }
            VisibleEvent::Reject | VisibleEvent::CounterAltRemove { .. } => {}
            VisibleEvent::EndTurn => {
                let l = &mut self.ledgers[a];
                l.turns_ended += 1;
                let dev_count = ev.before.dev_count[a];
                let bought = self.ledgers[a]
                    .purchases
                    .last()
                    .filter(|p| p.turn == ev.turn)
                    .map_or(0, |p| p.count);
                // 手番の開始時点の機会と終了時点の機会の「良い方」を採る。
                // 盗賊に塞がれていたのが 7 で外れた場合などに、機会が無かった扱いにしない
                let start = self.turn_start_opportunity[a].unwrap_or(opp);
                let merged = Opportunity {
                    blocked_pips: start.blocked_pips.max(opp.blocked_pips),
                    army_gain: start.army_gain || opp.army_gain,
                    can_build_road: start.can_build_road || opp.can_build_road,
                    road_race: start.road_race || opp.road_race,
                    bank_has_two: start.bank_has_two || opp.bank_has_two,
                    ..opp
                };
                self.turns.push(TurnRecord {
                    player: ev.actor,
                    turn: ev.turn,
                    dev_count,
                    bought_this_turn: bought,
                    played: self.played_this_turn,
                    opportunity: merged,
                });
                self.played_this_turn = None;
                self.turn_start_opportunity = [None; MAX_PLAYERS];
                self.open_trade = None;
            }
            _ => {}
        }
        true
    }

    fn note_play(&mut self, a: usize, turn: u32, c: DevCard) {
        let l = &mut self.ledgers[a];
        l.plays.push((turn, c));
        l.played_dev[c.idx()] += 1;
        self.played_this_turn = Some(c);
    }

    /// その人が買った発展カードの総数
    pub fn bought_total(&self, p: PlayerId) -> u8 {
        self.ledgers[p as usize].purchases.iter().map(|p| p.count).sum()
    }
}
