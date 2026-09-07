//! 本番 `Game` から席ごとの観測への投影（信頼境界）。
//!
//! ここだけが本番状態を読む。出て行くのは [`Observation`] と [`ObservedEvent`] の値だけで、
//! どちらも `Game` への参照や本番の乱数状態を含まない。

use crate::action::{Action, ActionRecord, Bundle, Outcome, Prompt, NUM_DEV_KINDS, EMPTY};
use crate::board::{PlayerId, NUM_RESOURCES};
use crate::game::{Game, MAX_PLAYERS};
use crate::observation::*;
use crate::rng::Rng;
use crate::topology::{EdgeId, NodeId, Topology, NUM_EDGES, NUM_NODES};

/// 秘密を消した局面を作る。
///
/// 相手の手札・発展カード・今手番に買った札の内訳・山の内訳・本番 RNG を落とす。
/// 自分の分も落とす（本人の私的状態は [`OwnState`] で別に運ぶ）。
pub fn redact(g: &Game, profile: ObservationProfile) -> Game {
    let mut r = g.clone();
    for q in 0..MAX_PLAYERS {
        r.players[q].hand = EMPTY;
        r.players[q].dev = [0; NUM_DEV_KINDS];
        r.players[q].dev_bought_this_turn = [0; NUM_DEV_KINDS];
    }
    r.dev_deck = [0; NUM_DEV_KINDS];
    // 本番の乱数状態を渡さない。定数にしておけば「同じ観測なら同じ値」になる
    r.rng_dice = Rng::new(0);
    r.rng_dev = Rng::new(0);
    r.rng_steal = Rng::new(0);
    if !profile.bank_exact {
        r.bank = EMPTY;
    }
    r
}

fn public_players(g: &Game) -> [PublicPlayer; MAX_PLAYERS] {
    let mut out = [PublicPlayer::default(); MAX_PLAYERS];
    for q in 0..g.n() {
        let ps = &g.players[q];
        out[q] = PublicPlayer {
            hand_size: ps.hand_size(),
            dev_count: ps.dev_count(),
            dev_bought_this_turn: ps.dev_bought_this_turn.iter().sum(),
            played_dev: ps.played_dev,
            public_vp: g.public_vp(q as PlayerId),
        };
    }
    out
}

/// 席 `viewer` の観測を作る。`legal` はその席に実際に提示できる合法手。
pub fn observe(g: &Game, viewer: PlayerId, legal: &[Action], profile: ObservationProfile, seq: u64) -> Observation {
    let me = &g.players[viewer as usize];
    Observation {
        viewer,
        rules_version: RULES_VERSION,
        profile,
        seq,
        public: PublicState {
            game: redact(g, profile),
            players: public_players(g),
            dev_deck_left: g.dev_deck.iter().sum(),
            bank: if profile.bank_exact { Some(g.bank) } else { None },
        },
        own: OwnState {
            hand: me.hand,
            dev: me.dev,
            dev_bought_this_turn: me.dev_bought_this_turn,
            actual_vp: g.actual_vp(viewer),
        },
        legal: legal.to_vec(),
    }
}

/// 盗賊がそのプレイヤーの産出を止めている pip
fn blocked_pips(g: &Game, p: PlayerId) -> u16 {
    let topo = Topology::get();
    let t = g.board.robber;
    if g.board.tile_resource[t as usize].is_none() {
        return 0;
    }
    let pips = g.board.tile_pips(t) as u16;
    let mut n = 0u16;
    for node in topo.tile_nodes[t as usize] {
        if let Some(b) = g.board.building[node as usize] {
            if b.owner == p {
                n += match b.kind {
                    crate::board::BuildingKind::Settlement => 1,
                    crate::board::BuildingKind::City => 2,
                };
            }
        }
    }
    pips * n
}

/// 今すぐ建てられる頂点の数（自分の道に接し、距離ルールを満たす空き頂点）
fn buildable_nodes(g: &Game, p: PlayerId) -> u8 {
    (0..NUM_NODES)
        .filter(|&n| g.can_place_settlement(n as NodeId, p, false))
        .count() as u8
}

/// 出来事の直前の公開状態の要点
pub fn context(g: &Game, profile: ObservationProfile) -> EventContext {
    let mut c = EventContext {
        robber: g.board.robber,
        largest_army_owner: g.largest_army_owner,
        longest_road_owner: g.longest_road_owner,
        bank: if profile.bank_exact { Some(g.bank) } else { None },
        dev_played_this_turn: g.dev_played_this_turn,
        rolled: g.rolled,
        ..EventContext::default()
    };
    for q in 0..g.n() {
        let p = q as PlayerId;
        let ps = &g.players[q];
        c.blocked_pips[q] = blocked_pips(g, p);
        c.played_knights[q] = ps.played_knights();
        c.longest_road[q] = ps.longest_road;
        c.roads_left[q] = ps.roads_left;
        c.can_build_road[q] = ps.roads_left > 0 && (0..NUM_EDGES).any(|e| g.can_build_road(e as EdgeId, p));
        c.settlements_left[q] = ps.settlements_left;
        c.cities_left[q] = ps.cities_left;
        c.buildable_nodes[q] = buildable_nodes(g, p);
        c.hand_size[q] = ps.hand_size();
        c.dev_count[q] = ps.dev_count();
        c.public_vp[q] = g.public_vp(p);
    }
    c
}

fn aftermath(g: &Game, profile: ObservationProfile) -> EventAftermath {
    let mut a = EventAftermath {
        dev_deck_left: g.dev_deck.iter().sum(),
        bank: if profile.bank_exact { Some(g.bank) } else { None },
        longest_road_owner: g.longest_road_owner,
        largest_army_owner: g.largest_army_owner,
        turn_player: g.turn_player,
        to_act: g.to_act,
        prompt: g.prompt,
        winner: g.winner,
        ..EventAftermath::default()
    };
    for q in 0..g.n() {
        let ps = &g.players[q];
        a.hand_size[q] = ps.hand_size();
        a.dev_count[q] = ps.dev_count();
        a.dev_bought_this_turn[q] = ps.dev_bought_this_turn.iter().sum();
        a.public_vp[q] = g.public_vp(q as PlayerId);
    }
    a
}

fn diff(before: &Bundle, after: &Bundle) -> Bundle {
    let mut d = EMPTY;
    for i in 0..NUM_RESOURCES {
        d[i] = after[i].saturating_sub(before[i]);
    }
    d
}

/// 行動 `rec` を、席 `viewer` から見える出来事に投影する。
///
/// `before` は行動の直前、`after` は直後の本番状態。
pub fn project_event(
    before: &Game,
    after: &Game,
    rec: &ActionRecord,
    viewer: PlayerId,
    profile: ObservationProfile,
    seq: u64,
) -> ObservedEvent {
    let actor = rec.actor;
    let is_actor = viewer == actor;
    let payload = match (rec.action, rec.result) {
        (Action::SetupSettlement(node), _) => VisibleEvent::SetupSettlement {
            node,
            gained: diff(&before.players[actor as usize].hand, &after.players[actor as usize].hand),
        },
        (Action::SetupRoad(edge), _) => VisibleEvent::SetupRoad { edge },
        (Action::Roll, Outcome::Dice(d1, d2)) => {
            let mut produced = [EMPTY; MAX_PLAYERS];
            for q in 0..before.n() {
                produced[q] = diff(&before.players[q].hand, &after.players[q].hand);
            }
            VisibleEvent::Roll { d1, d2, produced }
        }
        (Action::Roll, _) => VisibleEvent::Hidden,
        (Action::Discard(b), _) => VisibleEvent::Discard {
            count: b.iter().sum(),
            bundle: if is_actor || profile.discard_public { Some(b) } else { None },
        },
        (Action::MoveRobber { tile, victim }, out) => {
            let st = match out {
                Outcome::Stole(s) => s,
                _ => None,
            };
            let party = is_actor || victim == Some(viewer);
            VisibleEvent::RobberMoved {
                tile,
                victim,
                stole_any: st.is_some(),
                stolen: if party || profile.steal_kind_public { st } else { None },
            }
        }
        (Action::BuildRoad(edge), _) => VisibleEvent::BuildRoad {
            edge,
            free: before.prompt == Prompt::FreeRoad,
        },
        (Action::BuildSettlement(node), _) => VisibleEvent::BuildSettlement { node },
        (Action::BuildCity(node), _) => VisibleEvent::BuildCity { node },
        (Action::BuyDevCard, out) => {
            let card = match out {
                Outcome::Drew(c) if is_actor => Some(c),
                _ => None,
            };
            VisibleEvent::BuyDev { card }
        }
        (Action::PlayKnight, _) => VisibleEvent::PlayKnight,
        (Action::PlayRoadBuilding, _) => VisibleEvent::PlayRoadBuilding,
        (Action::PlayYearOfPlenty(a, b), _) => VisibleEvent::PlayYearOfPlenty { a, b },
        (Action::PlayMonopoly(res), _) => {
            let mut taken = [0u8; MAX_PLAYERS];
            for q in 0..before.n() {
                if q as PlayerId != actor {
                    taken[q] = before.players[q].hand[res.idx()];
                }
            }
            VisibleEvent::PlayMonopoly { res, taken }
        }
        (Action::MaritimeTrade { give, count, take }, _) => VisibleEvent::Maritime { give, count, take },
        (Action::OfferTrade { give, want }, _) => VisibleEvent::Offer { give, want },
        (Action::AcceptTrade, _) => VisibleEvent::Accept,
        (Action::RejectTrade, _) => VisibleEvent::Reject,
        (Action::CounterAlt { give, want }, _) => {
            if is_actor || profile.counter_drafts_public {
                VisibleEvent::CounterAlt { give, want }
            } else {
                VisibleEvent::Hidden
            }
        }
        (Action::CounterAltRemove(index), _) => {
            if is_actor || profile.counter_drafts_public {
                VisibleEvent::CounterAltRemove { index }
            } else {
                VisibleEvent::Hidden
            }
        }
        (Action::CounterOffer { give, want }, _) => VisibleEvent::Counter { give, want },
        (Action::ConfirmTrade(partner), _) => {
            let t = before.trade.expect("交易中でない");
            VisibleEvent::Confirm { partner, give: t.give, want: t.want }
        }
        (Action::AcceptCounter { from, alt }, _) => {
            let t = before.trade.expect("交易中でない");
            let (pg, prg) = t.counters[from as usize][alt as usize].expect("その候補は無い");
            VisibleEvent::AcceptCounter { partner: from, alt, partner_gives: pg, proposer_gives: prg }
        }
        (Action::CancelTrade, _) => VisibleEvent::Cancel,
        (Action::EndTurn, _) => VisibleEvent::EndTurn,
    };
    ObservedEvent {
        seq,
        viewer,
        actor,
        prompt: before.prompt,
        turn: before.turn,
        turn_player: before.turn_player,
        num_players: before.num_players,
        payload,
        before: context(before, profile),
        after: aftermath(after, profile),
    }
}
